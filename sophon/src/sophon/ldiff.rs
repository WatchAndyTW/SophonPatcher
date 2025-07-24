use std::fs::{self, File};
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use anyhow::Result;
use memmap2::MmapOptions;
use crate::proto::sophon::Asset;

/// Function to process a single asset data
pub async fn ldiff_file(
    data: &Asset,
    asset_name: &str,
    ldiffs_dir: &Path,
    output_dir: &Path,
) -> Result<()> {
    // Check if ldiff file exists
    let path = ldiffs_dir.join(&data.chunk_file_name);
    if !path.exists() {
        return Err(anyhow::anyhow!("{} does not exist", data.chunk_file_name));
    }

    // Open the file with error handling
    let file = match File::open(path.clone()) {
        Ok(file) => file,
        #[allow(unused_variables)]
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error opening file {}: {}", path.display(), e);
            return Err(anyhow::anyhow!("Error opening file {}: {}", path.display(), e));
        }
    };

    let file_size = match file.metadata() {
        Ok(metadata) => metadata.len(),
        #[allow(unused_variables)]
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error getting file size for {}: {}", path.display(), e);
            return Err(anyhow::anyhow!("Error getting file size for {}: {}", path.display(), e));
        }
    };

    let buffer = if file_size > 10 * 1024 * 1024 && data.hdiff_file_size > 1 * 1024 * 1024 {
        // For large files, use memory mapping
        match unsafe { MmapOptions::new().map(&file) } {
            Ok(mmap) => {
                let start = data.hdiff_file_in_chunk_offset as usize;
                let end = start + data.hdiff_file_size as usize;

                if end <= mmap.len() {
                    // Create a new buffer with the slice from mmap
                    let mut buffer = Vec::with_capacity(data.hdiff_file_size as usize);
                    buffer.extend_from_slice(&mmap[start..end]);
                    Some(buffer)
                } else {
                    #[cfg(debug_assertions)]
                    eprintln!("Error: Requested range exceeds file size for {}", path.display());
                    None
                }
            },
            #[allow(unused_variables)]
            Err(e) => {
                eprintln!("Error memory-mapping file {}: {}", path.display(), e);
                // Fall back to buffered reading
                read_buffer_with_bufreader(
                    &file,
                    data.hdiff_file_in_chunk_offset as i32,
                    data.hdiff_file_size as i32
                )
            }
        }
    } else {
        // For smaller files, use buffered reader
        read_buffer_with_bufreader(
            &file,
            data.hdiff_file_in_chunk_offset as i32,
            data.hdiff_file_size as i32
        )
    };

    // If buffer is None, return early
    let buffer = match buffer {
        Some(buf) => buf,
        None => return Err(anyhow::anyhow!("Error processing file {}", path.display())),
    };

    // Write assembled asset with proper error handling
    // TODO: HSR diffing empty file issue, fixed for patcher already
    let extension = if data.original_file_size == 0 { "" } else { ".hdiff" };
    let asset_path = output_dir.join(format!("{}{}", asset_name, extension));

    // Create parent directories if needed
    if let Some(parent) = asset_path.parent() {
        if !parent.exists() {
            #[allow(unused_variables)]
            if let Err(e) = fs::create_dir_all(parent) {
                #[cfg(debug_assertions)]
                eprintln!("Error creating directory {}: {}", parent.display(), e);
                return Err(anyhow::anyhow!("Error creating directory {}: {}", parent.display(), e));
            }
        }
    }

    // Write the file
    match fs::write(&asset_path, &buffer) {
        Ok(_) => Ok(()),
        #[allow(unused_variables)]
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error writing file {}: {}", asset_path.display(), e);
            Err(anyhow::anyhow!("Error writing file {}: {}", asset_path.display(), e))
        }
    }
}

/// Helper function to read a specific section of a file using BufReader
fn read_buffer_with_bufreader(file: &File, offset: i32, size: i32) -> Option<Vec<u8>> {
    let mut reader = BufReader::with_capacity(128 * 1024, file);

    // Seek to the specified offset
    #[allow(unused_variables)]
    if let Err(e) = reader.seek(SeekFrom::Start(offset as u64)) {
        #[cfg(debug_assertions)]
        eprintln!("Error seeking to offset {}: {}", offset, e);
        return None;
    }

    // Read the specified number of bytes
    let mut buffer = vec![0; size as usize];
    match reader.read_exact(&mut buffer) {
        Ok(_) => Some(buffer),
        #[allow(unused_variables)]
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("Error reading data: {}", e);
            None
        }
    }
}
