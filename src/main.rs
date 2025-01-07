use std::fs;
use std::io::Read;

use anyhow::Ok;
use clap::{Parser, Subcommand};

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
    HashFile,
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
        GitCmd::HashFile => todo!(),
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
        &file_hash[0..2],
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
