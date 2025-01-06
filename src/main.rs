#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;

fn main() {
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    let args: Vec<String> = env::args().collect();
    let command = &*args[1];

    match command {
        "init" => {
            let _ = fs::create_dir(".git");
            let _ = fs::create_dir(".git/objects");
            let _ = fs::create_dir(".git/refs");
            let _ = fs::write(".git/HEAD", "ref: refs/heads/master\n");
        }
        _ => eprintln!("unknown command {command}"),
    }
}
