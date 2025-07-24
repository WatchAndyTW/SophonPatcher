#![feature(once_cell_try)]

use std::env;
use std::path::Path;

mod util;
mod hpatchz;
mod action;
mod serialize;
mod extractor;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {
    let args: Vec<String> = env::args().collect();

    // Ask for input
    let buffer = args.get(1)
        .map(|s| s.clone())
        .unwrap_or_else(|| {
            println!("[Options]");
            println!("0 - Patch game by hdiff");
            println!("1 - Patch game by ldiff");
            println!("2 - Patch game by chunk");
            util::input("Please select action: ")
        });
    match buffer.as_str() {
        "0" => {
            let game_folder = args.get(2)
                .map(|s| s.clone())
                .unwrap_or_else(|| util::input("Please enter game folder: "));
            if let Err(err) = action::hdiff(
                &Path::new(&game_folder),
                args.get(3)
                    .map(|s| s.clone())
                    .unwrap_or_else(|| util::input("Please enter hdiff file name: ")),
            ).await {
                println!("{}", err);
            }
        },
        "1" => {
            let game_folder = args.get(2)
                .map(|s| s.clone())
                .unwrap_or_else(|| util::input("Please enter game folder: "));
            let ldiff_folder = args.get(3)
                .map(|s| s.clone())
                .unwrap_or_else(|| util::input("Please enter ldiff folder: "));
            if let Err(err) = action::ldiff(
                &Path::new(&game_folder),
                ldiff_folder,
                args.get(4)
                    .map(|s| s.clone())
                    .unwrap_or_else(|| util::input("Please enter manifest name: ")),
            ).await {
                println!("{}", err);
            }
        },
        "2" => {
            let game_folder = args.get(2)
                .map(|s| s.clone())
                .unwrap_or_else(|| util::input("Please enter game folder: "));
            let chunk_folder = args.get(3)
                .map(|s| s.clone())
                .unwrap_or_else(|| util::input("Please enter chunk folder: "));
            if let Err(err) = action::chunk(
                &Path::new(&game_folder),
                chunk_folder,
                args.get(4)
                    .map(|s| s.clone())
                    .unwrap_or_else(|| util::input("Please enter manifest name: ")),
            ).await {
                println!("{}", err);
            }
        },
        _ => {
            println!("Unknown command.");
        }
    }

    // Pause
    util::input("Press Enter to continue...");
}
