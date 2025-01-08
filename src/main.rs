use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::{fs, path::PathBuf};

use anyhow::{anyhow, Context, Ok};
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
        GitCmd::Init => {
            init()?;
        }
        GitCmd::CatFile { pretty_print, hash } => {
            anyhow::ensure!(pretty_print, "must pass -p flag");
            cat_file(&hash)?;
        }
        GitCmd::HashObject { write, path } => {
            let sha1sum = hash_object(write, &path)?;
            let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
            println!("{hex_string}");
        }
        GitCmd::LsTree { name_only, hash } => {
            ls_tree(name_only, &hash)?;
        }
        GitCmd::WriteTree => {
            let sha1sum = write_tree(".")?;
            let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
            println!("{hex_string}");
        }
    }
    Ok(())
}

fn init() -> anyhow::Result<()> {
    fs::create_dir(".git").context("failed to create the git directory")?;
    fs::create_dir(".git/objects").context("failed to create the objects database")?;
    fs::create_dir(".git/refs").context("failed to create the refs")?;
    fs::write(".git/HEAD", "ref: refs/heads/master\n").context("failed to specify the HEAD")?;
    Ok(())
}

fn cat_file(hash: &str) -> anyhow::Result<String> {
    let object = std::fs::File::open(object_path(hash)).context("failed to find the hash file")?;
    let mut zlib_decoder = flate2::read::ZlibDecoder::new(object);

    let mut buf = Vec::new();
    zlib_decoder.read_to_end(&mut buf)?;
    let mut iter = buf.iter();

    // discard type and size
    let _ = iter
        .find(|c| **c == b'\0')
        .ok_or(anyhow!("malformed object"))?;
    let data: Vec<u8> = iter.copied().collect();
    let data = String::from_utf8(data)?;
    Ok(data)
}

fn hash_object(write: bool, path: &str) -> anyhow::Result<Vec<u8>> {
    let mut object = std::fs::File::open(path).context("failed to open the file to hash")?;

    let mut buf = Vec::new();
    let size = object
        .read_to_end(&mut buf)
        .context("failed to read from the file")?;
    let header = format!("blob {size}\0");
    let sha1sum = hash_raw_object(&header, &buf);

    if !write {
        return Ok(sha1sum);
    }

    let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();
    ensure_object_dir(&hex_string)?;

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf).unwrap();

    fs::write(object_path(&hex_string), compressed_buf).unwrap();
    Ok(sha1sum)
}

fn ls_tree(name_only: bool, hash: &str) -> anyhow::Result<()> {
    let tree =
        std::fs::File::open(object_path(hash)).context(format!("failed to open object {hash}"))?;
    let mut zlib_decoder = flate2::read::ZlibDecoder::new(tree);

    let mut buf = Vec::new();
    zlib_decoder
        .read_to_end(&mut buf)
        .context(format!("failed to decode object {hash}"))?;

    #[derive(Debug)]
    struct Node {
        #[allow(dead_code)]
        mode: String,
        name: String,
        #[allow(dead_code)]
        hash: Vec<u8>,
    }

    let mut nodes = Vec::new();
    let mut ptr = buf
        .iter()
        .position(|c| *c == b'\0')
        .ok_or(anyhow!("malformed object {hash}"))?;
    while let Some(mode_end_index) = buf[ptr..].iter().position(|c| *c == b' ') {
        let mode = String::from_utf8(buf[ptr..ptr + mode_end_index].to_vec())?;
        ptr += mode_end_index + 1;

        if let Some(name_end_index) = buf[ptr..].iter().position(|c| *c == b'\0') {
            let name = String::from_utf8(buf[ptr..ptr + name_end_index].to_vec())?;
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
    Ok(())
}

fn write_tree(path: &str) -> anyhow::Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(path)
        .context(format!("failed to read dir {path}"))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let metadata = entry.metadata()?;
        let name = entry.file_name().to_str().unwrap_or_default().to_owned();
        if name == ".git" {
            continue;
        }

        let (mode, hash) = if metadata.is_dir() {
            (
                0o40000,
                write_tree(entry.path().to_str().unwrap_or_default())?,
            )
        } else {
            let mode = match metadata.is_file() {
                true => metadata.mode(),
                false => 0o120_000,
            };
            (
                mode,
                hash_object(true, entry.path().to_str().unwrap_or_default())?,
            )
        };
        buf.extend(format!("{mode:o} {name}\0").as_bytes());
        buf.extend(hash);
    }
    let header = format!("tree {}\0", buf.len());
    let sha1sum = hash_raw_object(&header, &buf);
    let hex_string: String = sha1sum.iter().map(|byte| format!("{:02x}", byte)).collect();

    let mut blob = header.as_bytes().to_vec();
    blob.extend(buf);
    let mut zlib_encoder = flate2::read::ZlibEncoder::new(&blob[..], flate2::Compression::none());
    let mut compressed_buf = Vec::new();
    zlib_encoder.read_to_end(&mut compressed_buf)?;

    ensure_object_dir(&hex_string)?;
    fs::write(object_path(&hex_string), compressed_buf)?;
    Ok(sha1sum)
}

fn object_path(hash: &str) -> PathBuf {
    let mut p = PathBuf::from(".git/objects");
    p.push(&hash[..2]);
    p.push(&hash[2..]);
    p
}
fn ensure_object_dir(hash: &str) -> anyhow::Result<()> {
    fs::create_dir_all(format!(".git/objects/{}", &hash[..2]))
        .context(format!("failed to create object directory for {hash}"))
}

fn hash_raw_object(header: &str, body: &[u8]) -> Vec<u8> {
    let mut hasher = sha1::Sha1::new();
    hasher.update(header);
    hasher.update(body);
    let sha1sum = hasher.finalize();
    sha1sum.to_vec()
}
