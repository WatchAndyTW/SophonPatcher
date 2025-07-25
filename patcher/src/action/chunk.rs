use std::path::Path;
use anyhow::{anyhow, Result};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use tokio::fs;
use sophon::proto::chunk::SophonChunkProto;
use sophon::sophon::chunk_diff;
use crate::serialize::PkgVersion;
use crate::util;

pub async fn chunk(game_path: &Path, chunk_folder: String, manifest_name: String) -> Result<()> {
    println!();

    let chunk_path = game_path.join(chunk_folder);
    if !chunk_path.exists() {
        return Err(anyhow!("{:?} does not exist", chunk_path));
    }

    // Read manifest
    let manifest = SophonChunkProto::from(
        game_path.join(&manifest_name).to_string_lossy().to_string()
    )?;

    // Potentially memory leak game path
    let game_path_owned = game_path.to_path_buf();
    let game_path_static: &'static Path = Box::leak(game_path_owned.into_boxed_path());

    // Extract chunks
    chunk_diff(&manifest, game_path_static, &chunk_path, Some(None)).await?;

    // Verify file integrity
    let verify = util::input("Chunk patching done, verify file integrity? (Y/n) [n]: ");
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
    }

    // Delete ldiff folder
    let delete = util::input("Delete chunk folder and manifest? (Y/n) [Y]: ");
    if delete.to_lowercase() != "n" && delete.to_lowercase() != "no" {
        let _ = fs::remove_file(game_path.join(manifest_name)).await;
        let _ = fs::remove_dir_all(chunk_path).await;
    }

    Ok(())
}
