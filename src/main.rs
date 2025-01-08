use std::io::Read;
use std::os::unix::fs::MetadataExt;
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
    LsTree {
        #[clap(long)]
        name_only: bool,
        hash: String,
    },
    WriteTree,
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
        GitCmd::HashObject { write, path } => {
            let sha1sum = hash_object(write, &path);
            let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
            println!("{hex_string}");
        }
        GitCmd::LsTree { name_only, hash } => ls_tree(name_only, &hash),
        GitCmd::WriteTree => {
            let sha1sum = write_tree(".");
            let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
            println!("{hex_string}");
        }
    }
    Ok(())
}

fn init() {
    fs::create_dir(".git").unwrap();
    fs::create_dir(".git/objects").unwrap();
    fs::create_dir(".git/refs").unwrap();
    fs::write(".git/HEAD", "ref: refs/heads/master\n").unwrap();
}

fn cat_file(hash: &str) {
    let object = std::fs::File::open(object_path(hash)).unwrap();
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

fn hash_object(write: bool, path: &str) -> Vec<u8> {
    let mut object = std::fs::File::open(path).unwrap();

    let mut buf = Vec::new();
    let size = object.read_to_end(&mut buf).unwrap();
    let header = format!("blob {size}\0");
    let sha1sum = hash_raw_object(&header, &buf);

    if !write {
        return sha1sum;
    }

    let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
    ensure_object_dir(&hex_string);

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf).unwrap();

    fs::write(object_path(&hex_string), compressed_buf).unwrap();
    sha1sum
}

fn ls_tree(name_only: bool, hash: &str) {
    let tree = std::fs::File::open(object_path(hash)).unwrap();
    let mut zlib_decoder = flate2::read::ZlibDecoder::new(tree);

    let mut buf = Vec::new();
    zlib_decoder.read_to_end(&mut buf).unwrap();

    #[derive(Debug)]
    struct Node {
        mode: String,
        name: String,
        hash: Vec<u8>,
    }

    let mut nodes = Vec::new();
    let mut ptr = buf.iter().position(|c| *c == b'\0').unwrap();
    while let Some(mode_end_index) = buf[ptr..].iter().position(|c| *c == b' ') {
        let mode = String::from_utf8(buf[ptr..ptr + mode_end_index].to_vec()).unwrap();
        ptr += mode_end_index + 1;

        if let Some(name_end_index) = buf[ptr..].iter().position(|c| *c == b'\0') {
            let name = String::from_utf8(buf[ptr..ptr + name_end_index].to_vec()).unwrap();
            ptr += name_end_index + 1;
            let hash = buf[ptr..ptr + 20].to_vec();
            ptr += 20;
            nodes.push(Node { mode, name, hash });
        } else {
            panic!("malformed tree");
        }
    }
    if name_only {
        for node in nodes {
            println!("{}", node.name);
        }
    } else {
        print!("{nodes:?}");
    }
}

fn write_tree(path: &str) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();
    let mut entries: Vec<fs::DirEntry> =
        fs::read_dir(path).unwrap().filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let metadata = entry.metadata().unwrap();
        let name = entry.file_name().to_str().unwrap().to_owned();
        if name == ".git" {
            continue;
        }

        let (mode, hash) = if metadata.is_dir() {
            (0o40000, write_tree(entry.path().to_str().unwrap()))
        } else {
            let mode = match metadata.is_file() {
                true => metadata.mode(),
                false => 0o120_000,
            };
            (mode, hash_object(true, entry.path().to_str().unwrap()))
        };
        buf.extend(format!("{mode:o} {name}\0").as_bytes());
        buf.extend(hash)
    }
    let header = format!("tree {}\0", buf.len());
    let sha1sum = hash_raw_object(&header, &buf);
    let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf).unwrap();

    ensure_object_dir(&hex_string);
    fs::write(object_path(&hex_string), compressed_buf).unwrap();
    sha1sum
}

fn object_path(hash: &str) -> PathBuf {
    let mut p = PathBuf::from(".git/objects");
    p.push(&hash[..2]);
    p.push(&hash[2..]);
    p
}
fn ensure_object_dir(hash: &str) {
    fs::create_dir_all(format!(".git/objects/{}", &hash[..2])).unwrap();
}

fn hash_raw_object(header: &str, body: &[u8]) -> Vec<u8> {
    let mut hasher = sha1::Sha1::new();
    hasher.update(header);
    hasher.update(body);
    let sha1sum = hasher.finalize();
    sha1sum.to_vec()
}
