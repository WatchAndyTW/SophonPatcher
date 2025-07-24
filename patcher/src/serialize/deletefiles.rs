use std::fs::File;
use std::io::Read;
use std::path::Path;
use anyhow::Result;

pub struct DeleteFiles;

impl DeleteFiles {
    pub fn from(path: &Path) -> Result<Vec<String>> {
        let mut file = File::open(path)?;

        // Read file into string
        let mut string = String::new();
        file.read_to_string(&mut string)?;

        // Deserialize string into json
        let vector = string
            .split("\n")
            .map(|s| s.to_string())
            .collect::<Vec<String>>();
        Ok(vector)
    }
}