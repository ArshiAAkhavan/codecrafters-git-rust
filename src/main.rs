#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;
use std::io::Read;
use std::path::PathBuf;

fn main() {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    let args: Vec<String> = env::args().collect();
    let command = &*args[1];

    match command {
        "init" => init(),
        "cat-file" => cat_file(&args[2], &args[3]),
        _ => eprintln!("unknown command {command}"),
    }
}

fn init() {
    fs::create_dir(".git").unwrap();
    fs::create_dir(".git/objects").unwrap();
    fs::create_dir(".git/refs").unwrap();
    fs::write(".git/HEAD", "ref: refs/heads/master\n").unwrap();
}

fn cat_file(flag: &str, file_hash: &str) {
    match flag {
        "-p" => {
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
        _ => {
            eprintln!("unknown flag {flag}")
        }
    }
}
