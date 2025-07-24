use std::fs::File;
use std::io::Read;
use std::path::Path;
use serde::Deserialize;
use anyhow::{anyhow, Result};

#[derive(Deserialize)]
pub struct HDiffMap {
    pub diff_map: Vec<HDiffData>,
}

#[derive(Deserialize)]
pub struct HDiffData {
    pub source_file_name: String,
    pub target_file_name: String,
    pub patch_file_name: String,
}

impl HDiffMap {
    pub fn from(path: &Path) -> Result<HDiffMap> {
        let mut file = File::open(path)?;

        // Read file into buffer
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)?;

        // Deserialize buffer into json
        serde_json::from_slice(&buffer).map_err(|e| anyhow!(e))
    }
}