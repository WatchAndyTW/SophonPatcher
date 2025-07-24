use std::fs::File;
use std::io::Read;
use std::path::Path;
use anyhow::{Result};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct HDiffFiles {
    #[serde(rename = "remoteName")]
    pub remote_file: String,
}

impl HDiffFiles {
    pub fn from(path: &Path) -> Result<Vec<HDiffFiles>> {
        let mut file = File::open(path)?;

        // Read file into string
        let mut string = String::new();
        file.read_to_string(&mut string)?;

        // Deserialize string into json
        let vector = string.split("\n")
            .filter_map(|s| {
                if let Ok(result) = serde_json::from_str::<HDiffFiles>(s) {
                    Some(result)
                } else {
                    None
                }
            })
            .collect::<Vec<HDiffFiles>>();
        Ok(vector)
    }
}