pub mod chunk;
pub mod sophon;

use std::fs::File;
use std::io::{BufReader, Read};
use prost::{DecodeError, Message};
use zstd::Decoder;
use crate::proto::chunk::SophonChunkProto;
use crate::proto::sophon::SophonManifestProto;

impl SophonChunkProto {
    pub fn from(path: String) -> Result<Self, DecodeError> {
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let mut decoder = Decoder::new(reader).unwrap();

        // Read file into buffer
        let mut buffer = Vec::new();
        decoder.read_to_end(&mut buffer).unwrap();

        // Parse the Protobuf message
        let proto = Self::decode(&*buffer);
        let Ok(proto) = proto else {
            return Err(proto.unwrap_err());
        };

        Ok(proto)
    }
}

impl SophonManifestProto {
    pub fn from(path: String) -> Result<Self, DecodeError> {
        let file = File::open(&path).unwrap();
        let reader = BufReader::new(file);
        let mut decoder = Decoder::new(reader).unwrap();

        // Read file into buffer
        let mut buffer = Vec::new();
        decoder.read_to_end(&mut buffer).unwrap();

        // Parse the Protobuf message
        let proto = Self::decode(&*buffer);
        let Ok(proto) = proto else {
            return Err(proto.unwrap_err());
        };

        Ok(proto)
    }
}
