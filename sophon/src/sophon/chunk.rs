use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use futures::future::join_all;
use indicatif::{ProgressBar, ProgressStyle};
use leveldb::db::Database;
use leveldb::iterator::Iterable;
use leveldb::options::{Options, ReadOptions};
use memmap2::MmapOptions;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use crate::proto::chunk::SophonChunkProto;

pub async fn chunk_diff(
    manifest: &SophonChunkProto,
    output_path: &'static Path,
    chunk_path: &Path,
    progress_bar: Option<Option<ProgressBar>>,
) -> Result<()> {
    // Make chunk caches
    let mut cache_list: HashMap<String, i64> = HashMap::new();
    manifest.assets.iter().for_each(|asset| {
        asset.asset_chunks.iter().for_each(|chunk| {
            cache_list.insert(chunk.chunk_name.clone(), chunk.chunk_size_decompressed);
        });
    });

    // Check for chunk path's existence
    if !chunk_path.exists() {
        return Err(anyhow!("[Error] Chunk directory does not exist"));
    }

    // Use parallel processing for the chunks
    let chunk_entries: Vec<_> = match fs::read_dir(chunk_path) {
        Ok(dir) => dir.filter_map(Result::ok)
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect(),
        Err(e) => {
            return Err(anyhow!("[Error] Failed reading chunk directory: {}", e));
        }
    };

    // Open database
    let database = chunk_entries
        .iter()
        .find(|entry| !entry.file_type().unwrap().is_dir())
        .map(|entry| {
            // Open the leveldb database first
            let leveldb_path = format!("{}_db", entry.file_name().to_string_lossy().into_owned());
            match Database::open(&entry.path().parent().unwrap_or(Path::new("")).join(leveldb_path), &Options::new()) {
                Ok(db) => Ok(db),
                Err(e) => {
                    return Err((entry.path().to_string_lossy().into_owned(), e));
                }
            }
        });
    let database = match database {
        Some(Ok(database)) => database,
        Some(Err((path, e))) => {
            return Err(anyhow!("[Error] Failed opening database {}: {}", path, e));
        }
        None => {
            return Err(anyhow!("[Error] No database found"));
        }
    };

    // Remove folders and create new ones
    let temp_path = output_path.join("chunk_tmp");
    tokio::fs::remove_dir_all(&temp_path).await.unwrap_or_default();
    tokio::fs::create_dir_all(&temp_path).await.unwrap_or_default();

    let mut bars: Vec<&Option<ProgressBar>> = Vec::new();

    // Process each chunk file in parallel
    chunk_entries.par_iter().for_each(|entry| {
        // Process database entries and collect what we need to extract
        let mut iter = database.iter(&ReadOptions::new());
        let mut extracted_chunks = Vec::new();

        while let Some((key, value)) = iter.next() {
            let key = match String::from_utf8(key) {
                Ok(k) => k,
                Err(_) => continue,
            };

            let value = match String::from_utf8(value).ok().and_then(|v| v.parse::<u64>().ok()) {
                Some(v) => v,
                None => continue,
            };

            // Check if file exists in cache list
            if let Some(&size) = cache_list.get(&key) {
                extracted_chunks.push((key, value, size));
            }
        }

        let pb = if let Some(_) = progress_bar {
            println!("Extracting chunk files");
            let progress_bar = ProgressBar::new(extracted_chunks.len() as u64);
            progress_bar.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            Some(progress_bar)
        } else {
            None
        };

        // Now process all the chunks from this file
        if !extracted_chunks.is_empty() {
            let file = match File::open(entry.path()) {
                Ok(file) => file,
                #[allow(unused_variables)]
                Err(e) => {
                    #[cfg(debug_assertions)]
                    eprintln!("Error opening file {}: {}", entry.path().display(), e);
                    return;
                }
            };

            let file_size = match file.metadata() {
                Ok(metadata) => metadata.len(),
                Err(_) => return,
            };

            // For large files, use memory mapping
            if file_size > 10 * 1024 * 1024 {
                match unsafe { MmapOptions::new().map(&file) } {
                    Ok(mmap) => {
                        for (key, offset, size) in extracted_chunks {
                            if offset as usize + size as usize <= mmap.len() {
                                let buffer = &mmap[offset as usize..(offset as usize + size as usize)];
                                let asset_path = temp_path.join(&key);

                                // Create parent directories if needed
                                if let Some(parent) = asset_path.parent() {
                                    if !parent.exists() {
                                        #[allow(unused_variables)]
                                        if let Err(e) = fs::create_dir_all(parent) {
                                            #[cfg(debug_assertions)]
                                            eprintln!(
                                                "Error creating directory {}: {}",
                                                parent.display(),
                                                e,
                                            );
                                            continue;
                                        }
                                    }
                                }

                                #[allow(unused_variables)]
                                if let Err(e) = fs::write(&asset_path, buffer) {
                                    #[cfg(debug_assertions)]
                                    eprintln!(
                                        "Error writing chunk file {}: {}",
                                        asset_path.display(),
                                        e,
                                    );
                                }

                                if let Some(pb) = &pb {
                                    pb.inc(1u64);
                                }
                            }
                        }
                    },
                    #[allow(unused_variables)]
                    Err(e) => {
                        #[cfg(debug_assertions)]
                        eprintln!("Error memory-mapping file {}: {}", entry.path().display(), e);
                        // Fall back to using BufReader for this file
                        process_with_bufreader(&entry.path(), &extracted_chunks, &pb);
                    }
                }
            } else {
                // For smaller files, use buffered reader
                process_with_bufreader(&entry.path(), &extracted_chunks, &pb);
            }
        }
    });

    if let Some(pb) = progress_bar.as_ref() {
        bars.push(pb);
    }

    // Now combine the extracted chunks into assets
    let mut all_tasks = Vec::new();

    // Make new progress bar
    let pb = Arc::new(Mutex::new({
        if progress_bar.is_some() {
            println!("Merging chunk files");
            let pb = ProgressBar::new(manifest.assets.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            Some(pb)
        } else {
            None
        }
    }));

    for asset in manifest.assets.clone() {
        let temp_path = temp_path.clone();
        let pb_clone = Arc::clone(&pb);
        let task_handle = tokio::spawn(async move {
            #[cfg(debug_assertions)]
            println!("[Chunk] Combining asset: {}", asset.asset_name);

            // Increase progress bar
            let progress_bar = pb_clone.lock().unwrap();
            if let Some(pb) = progress_bar.as_ref() {
                pb.inc(1);
            }

            // Estimate buffer size for pre-allocation
            let asset_chunks = asset.asset_chunks.clone();
            let estimated_size = asset_chunks.iter()
                .filter_map(|chunk| {
                    let path = temp_path.join(&chunk.chunk_name);
                    if path.exists() {
                        match fs::metadata(path) {
                            Ok(metadata) => Some(
                                chunk.chunk_on_file_offset as usize + metadata.len() as usize
                            ),
                            Err(_) => None,
                        }
                    } else {
                        None
                    }
                })
                .max()
                .unwrap_or(0);

            // Create a shared buffer with pre-allocation
            let buf = Arc::new(Mutex::new(Vec::with_capacity(estimated_size)));

            // Process chunks in parallel with rayon
            asset_chunks.par_iter().for_each(|chunk| {
                let path = temp_path.join(&chunk.chunk_name);
                if !path.exists() {
                    return;
                }

                // Read chunk data - handle different approaches based on file size
                let buffer = read_chunk_data(&path, chunk.chunk_name.as_str());
                if buffer.is_empty() {
                    return;
                }

                // Lock the buffer and copy data
                let mut buf_guard = match buf.lock() {
                    Ok(guard) => guard,
                    Err(_) => return,
                };

                let offset = chunk.chunk_on_file_offset as usize;
                if buf_guard.len() < offset + buffer.len() {
                    buf_guard.resize(offset + buffer.len(), 0);
                }

                buf_guard[offset..offset + buffer.len()].copy_from_slice(&buffer);
            });

            // Extract the buffer from Arc<Mutex<>>
            let final_buf = match Arc::try_unwrap(buf) {
                Ok(mutex) => mutex.into_inner().unwrap_or_default(),
                Err(arc) => match arc.lock() {
                    Ok(guard) => guard.clone(),
                    Err(_) => Vec::new(),
                },
            };

            // Write to file using buffered writer if buffer is not empty
            if !final_buf.is_empty() {
                let output_path = output_path.join(&asset.asset_name);

                // Create parent directories if needed
                if let Some(parent) = Path::new(&output_path).parent() {
                    if !parent.exists() {
                        #[allow(unused_variables)]
                        if let Err(e) = fs::create_dir_all(parent) {
                            #[cfg(debug_assertions)]
                            eprintln!(
                                "Error creating directory for {}: {}",
                                output_path.display(),
                                e,
                            );
                            return;
                        }
                    }
                }

                match File::create(&output_path) {
                    Ok(file) => {
                        let mut writer = BufWriter::with_capacity(
                            256 * 1024, file
                        );
                        #[allow(unused_variables)]
                        if let Err(e) = writer.write_all(&final_buf) {
                            #[cfg(debug_assertions)]
                            eprintln!("Error writing to {}: {}", output_path.display(), e);
                            return;
                        }
                        #[allow(unused_variables)]
                        if let Err(e) = writer.flush() {
                            #[cfg(debug_assertions)]
                            eprintln!("Error flushing buffer for {}: {}", output_path.display(), e);
                        }
                    },
                    #[allow(unused_variables)]
                    Err(e) => {
                        #[cfg(debug_assertions)]
                        eprintln!("Error creating file {}: {}", output_path.display(), e);
                    }
                }
            }
        });

        all_tasks.push(task_handle);
    }

    // Wait for all tasks to complete
    let _ = join_all(all_tasks).await;
    bars.push(&pb.lock().unwrap().clone());

    // Delete chunk folder
    tokio::fs::remove_dir_all(temp_path).await.unwrap_or_default();

    Ok(())
}

/// Helper function for processing with BufReader
fn process_with_bufreader(
    path: &Path,
    chunks: &[(String, u64, i64)],
    progress_bar: &Option<ProgressBar>,
) {
    let file = match File::open(path) {
        Ok(file) => file,
        #[allow(unused_variables)]
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error opening file {}: {}", path.display(), e);
            return;
        }
    };

    let mut reader = BufReader::with_capacity(128 * 1024, file);

    for (key, offset, size) in chunks {
        // Jump to offset in leveldb
        #[allow(unused_variables)]
        if let Err(e) = reader.seek(SeekFrom::Start(*offset)) {
            #[cfg(debug_assertions)]
            eprintln!("Error seeking to offset {} in file {}: {}", offset, path.display(), e);
            continue;
        }

        let mut buffer = vec![0; *size as usize];
        #[allow(unused_variables)]
        if let Err(e) = reader.read_exact(&mut buffer) {
            #[cfg(debug_assertions)]
            eprintln!("Error reading data for chunk {}: {}", key, e);
            continue;
        }

        let asset_path = Path::new("chunk_tmp").join(key);

        // Create parent directories
        if let Some(parent) = asset_path.parent() {
            if !parent.exists() {
                #[allow(unused_variables)]
                #[allow(unused_variables)]
                if let Err(e) = fs::create_dir_all(parent) {
                    #[cfg(debug_assertions)]
                    eprintln!("Error creating directory {}: {}", parent.display(), e);
                    continue;
                }
            }
        }

        #[allow(unused_variables)]
        if let Err(e) = fs::write(&asset_path, &buffer) {
            #[cfg(debug_assertions)]
            eprintln!("Error writing chunk file {}: {}", asset_path.display(), e);
        }

        if let Some(pb) = &progress_bar {
            pb.inc(1u64);
        }
    }
}

/// Helper function to read chunk data
#[allow(unused_variables)]
fn read_chunk_data(path: &Path, chunk_name: &str) -> Vec<u8> {
    // Error handling for file operations
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error opening chunk {}: {}", chunk_name, e);
            return Vec::new();
        }
    };

    let chunk_size = match file.metadata() {
        Ok(metadata) => metadata.len() as usize,
        Err(_) => return Vec::new(),
    };

    // Choose appropriate reading method based on file size
    if chunk_size > 1024 * 1024 { // 1MB threshold
        // Memory map for large files
        match unsafe { MmapOptions::new().map(&file) } {
            Ok(mmap) => {
                let mut buffer = Vec::with_capacity(mmap.len());
                buffer.extend_from_slice(&mmap[..]);
                buffer
            },
            #[allow(unused_variables)]
            Err(e) => {
                #[cfg(debug_assertions)]
                eprintln!("Error memory-mapping chunk {}: {}", chunk_name, e);
                // Fall back to buffered reading
                read_with_bufreader(file, chunk_size)
            }
        }
    } else {
        // Buffered reader for smaller files
        read_with_bufreader(file, chunk_size)
    }
}

fn read_with_bufreader(file: File, capacity: usize) -> Vec<u8> {
    let mut buffer = Vec::with_capacity(capacity);
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    match reader.read_to_end(&mut buffer) {
        Ok(_) => buffer,
        Err(_) => Vec::new(),
    }
}
