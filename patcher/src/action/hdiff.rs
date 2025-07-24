use std::path::Path;
use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tokio::fs;
use crate::extractor::ArchiveExtractor;
use crate::hpatchz::HPatchZ;
use crate::serialize::{DeleteFiles, HDiffData, HDiffFiles, HDiffMap, PkgVersion};
use crate::util;

pub async fn hdiff(game_path: &Path, hdiff_file: String) -> Result<()> {
    println!();

    let hdiff_path = game_path.join(&hdiff_file);
    if !hdiff_path.exists() {
        return Err(anyhow!("{:?} does not exist", hdiff_file));
    }

    // Make progress bar
    println!("Extracting {}", hdiff_path.file_name().unwrap().to_string_lossy());
    let mut bars: Vec<Option<ProgressBar>> = Vec::new();
    let mut progress_bar: Option<ProgressBar> = None;

    // Extract hdiff file
    ArchiveExtractor::extract_with_progress(&hdiff_path, game_path, |cur, max| {
        let pb = progress_bar.get_or_insert_with(|| {
            let pb = ProgressBar::new(max as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            pb
        });
        pb.set_position(cur as u64);
    })?;
    bars.push(progress_bar);

    // Load hdiff map
    let hdiff_map = load_diff_map(&game_path).await?;

    // Create another progress bar
    println!("Patching game files");
    let progress_bar: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let err_list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Patch game files
    hdiff_map.diff_map.par_iter().for_each(|data| {
        // Increase progress bar
        let mut progress_bar = progress_bar.lock().unwrap();
        let pb = progress_bar.get_or_insert_with(|| {
            let pb = ProgressBar::new(hdiff_map.diff_map.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            pb
        });
        pb.inc(1u64);

        // Check if source file exist
        let source_path = game_path.join(&data.source_file_name);
        if !source_path.exists() {
            return;
        }

        // Check if patch file exist
        let patch_path = game_path.join(&data.patch_file_name);
        if !patch_path.exists() {
            return;
        }

        // Run hpatchz
        let target_path = game_path.join(&data.target_file_name);
        if let Err(_) = HPatchZ::apply_patch(&source_path, &patch_path, &target_path) {
            err_list.lock().unwrap().push(data.source_file_name.clone());
            return;
        }

        // Delete hdiff file
        std::fs::remove_file(patch_path).unwrap();
    });
    bars.push(progress_bar.lock().unwrap().clone());

    // Print error list
    err_list.lock().unwrap().iter().for_each(|file| {
        eprintln!("{file} failed to patch!");
    });

    // Remove files in deletefiles.txt
    if let Ok(deletes) = DeleteFiles::from(&game_path.join("deletefiles.txt")) {
        deletes.par_iter().for_each(|path| {
            let _ = std::fs::remove_file(game_path.join(path));
        })
    };

    // Remove hdiff entries files
    let _ = fs::remove_file(game_path.join("hdiffmap.json")).await;
    let _ = fs::remove_file(game_path.join("hdifffiles.txt")).await;
    let _ = fs::remove_file(game_path.join("deletefiles.txt")).await;

    // Cleanup hpatchz temp file
    HPatchZ::cleanup()?;

    // Verify file integrity
    let verify = util::input("Hdiff patching done, verify file integrity? (Y/n) [n]: ");
    if verify.to_lowercase() == "y" || verify.to_lowercase() == "yes" {
        let progress_bar: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
        let pkg_version = PkgVersion::from(&game_path.join("pkg_version"))?;
        pkg_version.par_iter().for_each(|file| {
            let mut progress_bar = progress_bar.lock().unwrap();
            let pb = progress_bar.get_or_insert_with(|| {
                let pb = ProgressBar::new(pkg_version.len() as u64);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                        .expect("Failed to set progress bar template")
                        .progress_chars("#>-"),
                );
                pb
            });
            pb.inc(1u64);

            let file_path = game_path.join(&file.remote_file);
            if let Ok(md5) = util::calculate_md5_hash(&file_path) {
                if md5.to_lowercase() != file.md5 {
                    println!(
                        "{} md5 hash does not match! Expected: {}, found: {}",
                        &file.remote_file,
                        &file.md5,
                        &md5,
                    );
                }
            }
        });
        bars.push(progress_bar.lock().unwrap().clone());
    }

    // Delete hdiff file
    let delete = util::input("Delete hdiff file? (Y/n) [Y]: ");
    if delete.to_lowercase() != "n" && delete.to_lowercase() != "no" {
        let _ = fs::remove_file(hdiff_path).await;
    }

    Ok(())
}

async fn load_diff_map(path: &Path) -> Result<HDiffMap> {
    if path.join("hdiffmap.json").exists() {
        HDiffMap::from(&path.join("hdiffmap.json"))
    } else if path.join("hdifffiles.txt").exists() {
        let files = HDiffFiles::from(&path.join("hdifffiles.txt"));
        Ok(HDiffMap {
            diff_map: files?.iter().map(|file| {
                HDiffData {
                    source_file_name: file.remote_file.clone(),
                    target_file_name: file.remote_file.clone(),
                    patch_file_name: format!("{}.hdiff", file.remote_file.clone()),
                }
            }).collect::<Vec<_>>(),
        })
    } else {
        Err(anyhow!("No hdiff entries map exist"))
    }
}
