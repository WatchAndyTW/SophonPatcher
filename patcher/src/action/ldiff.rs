use std::path::Path;
use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use tokio::fs;
use sophon::proto::sophon::SophonManifestProto;
use sophon::sophon::ldiff_file;
use crate::hpatchz::HPatchZ;
use crate::serialize::{HDiffData, PkgVersion};
use crate::util;

pub async fn ldiff(game_path: &Path, ldiff_folder: String, manifest_name: String) -> Result<()> {
    println!();

    let ldiff_path = game_path.join(ldiff_folder);
    if !ldiff_path.exists() {
        return Err(anyhow!("{:?} does not exist", ldiff_path));
    }

    // Read manifest
    let manifest = SophonManifestProto::from(
        game_path.join(&manifest_name).to_string_lossy().to_string()
    )?;

    // Make progress bar
    println!("Extracting hdiff files from ldiff");
    let mut bars: Vec<Option<ProgressBar>> = Vec::new();
    let mut progress_bar: Option<ProgressBar> = None;

    // Extract hdiff file
    let entries = ldiff_path.read_dir()?.collect::<Result<Vec<_>, _>>()?;
    for entry in ldiff_path.read_dir()? {
        let pb = progress_bar.get_or_insert_with(|| {
            let pb = ProgressBar::new(entries.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            pb
        });
        pb.inc(1u64);

        let asset_name = entry?.file_name().to_string_lossy().into_owned();
        let matching_assets = manifest.assets
            .par_iter()
            .filter_map(|asset_group| {
                if let Some(data) = &asset_group.asset_data {
                    let asset = data.assets
                        .iter()
                        .find(|asset| asset.chunk_file_name == asset_name);
                    if let Some(asset) = asset {
                        return Some((asset_group.asset_name.clone(), asset.clone()));
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        for (name, asset) in matching_assets {
            ldiff_file(
                &asset,
                &name,
                &ldiff_path,
                &game_path,
            ).await?;
        }
    }
    bars.push(progress_bar);

    // Make hdiff map
    let hdiff_map = make_diff_map(
        &manifest,
        entries.iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    ).await?;

    // Create another progress bar
    println!("Patching game files");
    let progress_bar: Arc<Mutex<Option<ProgressBar>>> = Arc::new(Mutex::new(None));
    let err_list: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // Patch game files
    hdiff_map.par_iter().for_each(|data| {
        // Increase progress bar
        let mut progress_bar = progress_bar.lock().unwrap();
        let pb = progress_bar.get_or_insert_with(|| {
            let pb = ProgressBar::new(hdiff_map.len() as u64);
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}")
                    .expect("Failed to set progress bar template")
                    .progress_chars("#>-"),
            );
            pb
        });
        pb.inc(1u64);

        // Check if patch file exist
        let patch_path = game_path.join(&data.patch_file_name);
        if !patch_path.exists() {
            return;
        }

        // Run hpatchz
        if !data.source_file_name.is_empty() {
            let source_path = game_path.join(&data.source_file_name);
            if !source_path.exists() {
                return;
            }

            let target_path = game_path.join(&data.target_file_name);
            if let Err(_) = HPatchZ::apply_patch(&source_path, &patch_path, &target_path) {
                err_list.lock().unwrap().push(data.target_file_name.clone());
                return;
            }

            if data.source_file_name != data.target_file_name {
                std::fs::remove_file(&source_path).unwrap();
            }
        } else {
            let target_path = game_path.join(format!("{}.1", data.target_file_name));
            if let Err(_) = HPatchZ::apply_patch_empty(&patch_path, &target_path) {
                err_list.lock().unwrap().push(data.target_file_name.clone());
                return;
            }

            let new_target = game_path.join(&data.target_file_name);
            std::fs::remove_file(&new_target).unwrap();
            std::fs::rename(&target_path, &new_target).unwrap();
        }

        // Delete hdiff file
        std::fs::remove_file(patch_path).unwrap();
    });
    bars.push(progress_bar.lock().unwrap().clone());

    // Print error list
    err_list.lock().unwrap().iter().for_each(|file| {
        eprintln!("{file} failed to patch!");
    });

    // Cleanup hpatchz temp file
    HPatchZ::cleanup()?;

    // Verify file integrity
    let verify = util::input("Ldiff patching done, verify file integrity? (Y/n) [n]: ");
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

    // Delete ldiff folder
    let delete = util::input("Delete ldiff folder and manifest? (Y/n) [Y]: ");
    if delete.to_lowercase() != "n" && delete.to_lowercase() != "no" {
        let _ = fs::remove_file(game_path.join(manifest_name)).await;
        let _ = fs::remove_dir_all(ldiff_path).await;
    }

    Ok(())
}

async fn make_diff_map(
    manifest: &SophonManifestProto,
    chunk_names: Vec<String>,
) -> Result<Vec<HDiffData>> {
    // Iterate all assets in proto
    let hdiff_files = manifest.assets.iter().filter_map(|asset| {
        let asset_name = asset.asset_name.clone();
        let asset_size = asset.asset_size.clone();
        if let Some(chunk) = asset.asset_data.clone() {
            let assets = chunk.assets
                .iter()
                .filter(|&asset| chunk_names.iter().any(|name| name == &asset.chunk_file_name))
                .filter_map(|asset| {
                    if asset.original_file_size != 0 {
                        Some(HDiffData {
                            source_file_name: asset.original_file_path.clone(),
                            target_file_name: asset_name.clone(),
                            patch_file_name: format!("{asset_name}.hdiff"),
                        })
                    } else if asset.hdiff_file_size != asset_size {
                        Some(HDiffData {
                            source_file_name: asset.original_file_path.clone(),
                            target_file_name: asset_name.clone(),
                            patch_file_name: asset_name.clone(),
                        })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            Some(assets)
        } else {
            None
        }
    }).collect::<Vec<_>>();

    Ok(hdiff_files.into_iter().flatten().collect())
}
