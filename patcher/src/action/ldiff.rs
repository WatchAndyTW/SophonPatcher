use std::path::Path;
use anyhow::{anyhow, Result};
use indicatif::ProgressBar;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use tokio::fs;
use sophon::proto::sophon::SophonManifestProto;
use crate::extractor::ArchiveExtractor;
use crate::hpatchz::HPatchZ;
use crate::serialize::{HDiffData, PkgVersion};
use crate::util;

pub async fn ldiff(
    game_path: &Path,
    ldiff_file: String,
) -> Result<()> {
    println!();

    let ldiff_file_path = game_path.join(&ldiff_file);
    if !ldiff_file_path.exists() {
        return Err(anyhow!("{:?} does not exist", ldiff_file_path));
    }
    let ldiff_path = game_path.join("ldiff");

    // Make progress bar
    println!("Extracting {}", ldiff_file_path.file_name().unwrap().to_string_lossy());
    let mut bars: Vec<ProgressBar> = Vec::new();
    let mut progress_bar: Option<ProgressBar> = None;

    // Extract hdiff file
    ArchiveExtractor::extract_with_progress(&ldiff_file_path, &game_path, |cur, max| {
        let pb = progress_bar.get_or_insert_with(|| {
            util::create_progress_bar(max as u64)
        });
        pb.set_position(cur as u64);
    })?;
    bars.push(progress_bar.unwrap());

    // Extract hdiff file
    println!("Extracting hdiff files from ldiff");
    for game_entry in game_path.read_dir()? {
        let entry = game_entry?;
        if entry.file_type()?.is_file() && entry.file_name().to_string_lossy().starts_with("manifest") {
            let manifest_name = entry.file_name().to_string_lossy().to_string();
            let manifest = match SophonManifestProto::from(
                game_path.join(&manifest_name).to_string_lossy().to_string()
            ) {
                Ok(manifest) => {
                    manifest
                }
                Err(_) => {
                    continue;
                }
            };

            let entries = ldiff_path.read_dir()?.collect::<Result<Vec<_>, _>>()?;
            let pb = util::create_progress_bar(entries.len() as u64);
            for entry in ldiff_path.read_dir()? {
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
                                let asset_name = asset_group.asset_name.clone();
                                let asset_size = asset_group.asset_size.clone();
                                return Some((asset_name, asset_size, asset.clone()));
                            }
                        }
                        None
                    })
                    .collect::<Vec<_>>();
                for (asset_name, asset_size, asset) in matching_assets {
                    sophon::sophon::ldiff_file(
                        &asset,
                        &asset_name,
                        asset_size,
                        &ldiff_path,
                        &game_path,
                    ).await?;
                }
            }
            bars.push(pb);

            // Make hdiff map
            println!("Patching game files");
            let hdiff_map = make_diff_map(
                &manifest,
                entries.iter()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
            ).await?;

            // Patch game files
            let pb = util::create_progress_bar(hdiff_map.len() as u64);
            hdiff_map.into_par_iter().for_each(|data| {
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
                        eprintln!("{} failed to patch!", &data.target_file_name);
                        std::fs::remove_file(&patch_path).unwrap();
                        return;
                    }

                    if data.source_file_name != data.target_file_name {
                        std::fs::remove_file(&source_path).unwrap();
                    }
                    std::fs::remove_file(patch_path).unwrap();
                } else {
                    let target_path = game_path.join(&data.target_file_name);
                    if let Err(_) = HPatchZ::apply_patch_empty(&patch_path, &target_path) {
                        eprintln!("{} failed to patch!", &data.target_file_name);
                        std::fs::remove_file(&patch_path).unwrap();
                        return;
                    }

                    std::fs::remove_file(&patch_path).unwrap();
                }
            });
            bars.push(pb);
        }
    }

    // Cleanup hpatchz temp file
    HPatchZ::cleanup()?;

    // Verify file integrity
    let verify = util::input("Ldiff patching done, verify file integrity? (Y/n) [n]: ");
    if verify.to_lowercase() == "y" || verify.to_lowercase() == "yes" {
        let pkg_version = PkgVersion::from(&game_path.join("pkg_version"))?;
        let pb = util::create_progress_bar(pkg_version.len() as u64);
        pkg_version.into_par_iter().for_each(|file| {
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
            } else {
                println!("{} does not exist!", &file.remote_file);
            }
        });
        bars.push(pb);
    }
    let _ = fs::remove_dir_all(ldiff_path).await;

    // Delete ldiff folder
    let delete = util::input("Delete ldiff folder and manifest? (Y/n) [Y]: ");
    if delete.to_lowercase() != "n" && delete.to_lowercase() != "no" {
        let _ = fs::remove_file(ldiff_file_path).await;
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
                    if asset.original_file_size != 0 || asset.hdiff_file_size != asset_size {
                        Some(HDiffData {
                            source_file_name: asset.original_file_path.clone(),
                            target_file_name: asset_name.clone(),
                            patch_file_name: format!("{asset_name}.hdiff"),
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
