use std::fs::File;
use std::io::Read;
use std::path::Path;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PkgVersion {
    #[serde(rename = "remoteName")]
    pub remote_file: String,
    pub md5: String,
}

impl PkgVersion {
    pub fn from(path: &Path) -> anyhow::Result<Vec<PkgVersion>> {
        let mut file = File::open(path)?;

        // Read file into string
        let mut string = String::new();
        file.read_to_string(&mut string)?;

        // Deserialize string into json
        let vector = string.split("\n")
            .filter_map(|s| {
                if let Ok(result) = serde_json::from_str::<PkgVersion>(s) {
                    Some(result)
                } else {
                    None
                }
            })
            .collect::<Vec<PkgVersion>>();
        Ok(vector)
    }
}