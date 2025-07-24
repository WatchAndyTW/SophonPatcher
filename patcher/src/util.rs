use std::fs::File;
use std::io;
use std::io::{BufReader, Read, Write};
use std::path::Path;
use md5::Context;

pub fn input(text: &str) -> String {
    print!("{text}");
    io::stdout().flush().unwrap();
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer).unwrap();
    buffer.trim().to_string()
}

/// Calculate MD5 hash of a file
///
/// # Arguments
/// * `file_path` - Path to the file to hash
///
/// # Returns
/// * `Result<String, io::Error>` - MD5 hash as hex string or IO error
pub fn calculate_md5_hash<P: AsRef<Path>>(file_path: P) -> Result<String, io::Error> {
    // Open the file
    let file = File::open(&file_path)?;

    // Create a buffered reader for efficient reading
    let mut reader = BufReader::new(file);

    // Create MD5 context
    let mut context = Context::new();

    // Buffer for reading chunks of the file
    let mut buffer = [0u8; 8192]; // 8KB buffer

    // Read file in chunks and update hash
    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break; // End of file
        }
        context.consume(&buffer[..bytes_read]);
    }

    // Compute final hash and convert to hex string
    let digest = context.compute();
    Ok(format!("{:x}", digest))
}
