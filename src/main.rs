use std::io::{BufReader, Read};
use std::{fs, path::PathBuf};

use anyhow::Ok;
use clap::{Parser, Subcommand};
use sha1::Digest;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct GitCli {
    #[command(subcommand)]
    cmd: GitCmd,
}

#[derive(Subcommand)]
enum GitCmd {
    Init,
    CatFile {
        #[clap(short)]
        pretty_print: bool,
        hash: String,
    },
    HashObject {
        #[clap(short)]
        write: bool,
        path: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = GitCli::parse();
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    match cli.cmd {
        GitCmd::Init => init(),
        GitCmd::CatFile { pretty_print, hash } => {
            anyhow::ensure!(pretty_print, "must pass -p flag");
            cat_file(&hash)
        }
        GitCmd::HashObject { write, path } => hash_object(write, &path),
    }
    Ok(())
}

fn init() {
    fs::create_dir(".git").unwrap();
    fs::create_dir(".git/objects").unwrap();
    fs::create_dir(".git/refs").unwrap();
    fs::write(".git/HEAD", "ref: refs/heads/master\n").unwrap();
}

fn cat_file(file_hash: &str) {
    let object = std::fs::File::open(format!(
        ".git/objects/{}/{}",
        &file_hash[..2],
        &file_hash[2..]
    ))
    .unwrap();
    let mut zlib_decoder = flate2::read::ZlibDecoder::new(object);

    let mut buf = Vec::new();
    zlib_decoder.read_to_end(&mut buf).unwrap();
    let mut iter = buf.iter();

    // discard type and size
    let _ = iter.find(|c| **c == b'\0').unwrap();
    let data: Vec<u8> = iter.map(|c| *c).collect();
    let data = String::from_utf8(data).unwrap();
    print!("{data}")
}

fn hash_object(write: bool, path: &str) {
    let mut object = std::fs::File::open(path).unwrap();

    let mut buf = Vec::new();
    let size = object.read_to_end(&mut buf).unwrap();
    let header = format!("blob {size}\0");
    let mut hasher = sha1::Sha1::new();
    hasher.update(&header);
    hasher.update(&buf);
    let sha1sum = hasher.finalize();
    let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
    print!("{hex_string}");

    if !write {
        return;
    }
    let mut object_path = PathBuf::from(".git/objects");
    object_path.push(&hex_string[..2]);
    fs::create_dir_all(&object_path).unwrap();

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf).unwrap();

    object_path.push(&hex_string[2..]);
    fs::write(object_path, compressed_buf).unwrap();
}
