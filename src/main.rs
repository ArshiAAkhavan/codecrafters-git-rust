use std::io::Read;
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
        GitCmd::LsTree { name_only, hash } => ls_tree(name_only, &hash),
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
    fs::create_dir_all(format!(".git/objects/{}", &hex_string[..2])).unwrap();

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf).unwrap();

    fs::write(object_path(&hex_string), compressed_buf).unwrap();
}

fn ls_tree(name_only: bool, hash: &str) {
    let tree = std::fs::File::open(object_path(hash)).unwrap();
    let mut zlib_decoder = flate2::read::ZlibDecoder::new(tree);

    let mut buf = Vec::new();
    zlib_decoder.read_to_end(&mut buf).unwrap();

    #[derive(Default, Debug)]
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

fn object_path(hash: &str) -> PathBuf {
    let mut p = PathBuf::from(".git/objects");
    p.push(&hash[..2]);
    p.push(&hash[2..]);
    p
}
