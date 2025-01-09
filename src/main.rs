use std::fmt::Write;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Ok};
use clap::{Parser, Subcommand};
use codecrafters_git as git;

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
    CommitTree {
        #[clap(short)]
        parent: String,
        #[clap(short)]
        message: String,
        tree: String,
    },
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
            let data = cat_file(&hash)?;
            print!("{data}")
        }
        GitCmd::HashObject { write, path } => {
            let sha1sum = hash_object(write, &path)?;
            println!("{}", display_hex(&sha1sum));
        }
        GitCmd::LsTree { name_only, hash } => {
            ls_tree(name_only, &hash)?;
        }
        GitCmd::WriteTree => {
            let sha1sum = write_tree(".")?;
            println!("{}", display_hex(&sha1sum));
        }
        GitCmd::CommitTree {
            parent,
            message,
            tree,
        } => {
            let sha1sum = commit_tree(parent, message, tree)?;
            println!("{}", display_hex(&sha1sum))
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
    let obj = git::Object::load(hash)?;
    let data = String::from_utf8(obj.body)?;
    Ok(data)
}

fn hash_object(write: bool, path: &str) -> anyhow::Result<[u8; 20]> {
    dbg!(path);
    let obj = git::Object::new_blob_from_file(path)?;

    if write {
        obj.persist()?;
    }
    Ok(obj.hash())
}

fn ls_tree(name_only: bool, hash: &str) -> anyhow::Result<()> {
    let obj = git::Object::load(hash)?;

    #[derive(Debug)]
    struct Node {
        #[allow(dead_code)]
        mode: String,
        name: String,
        #[allow(dead_code)]
        hash: Vec<u8>,
    }

    let mut nodes = Vec::new();
    let mut ptr = 0;
    while let Some(mode_end_index) = obj.body[ptr..].iter().position(|c| *c == b' ') {
        let mode = String::from_utf8(obj.body[ptr..ptr + mode_end_index].to_vec())?;
        ptr += mode_end_index + 1;

        if let Some(name_end_index) = obj.body[ptr..].iter().position(|c| *c == b'\0') {
            let name = String::from_utf8(obj.body[ptr..ptr + name_end_index].to_vec())?;
            ptr += name_end_index + 1;
            let hash = obj.body[ptr..ptr + 20].to_vec();
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

fn write_tree(path: &str) -> anyhow::Result<[u8; 20]> {
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
    let tree = git::Object::new(git::Kind::Tree, buf);
    dbg!("tree");
    tree.persist()?;
    Ok(tree.hash())
}

fn commit_tree(parent: String, message: String, tree: String) -> anyhow::Result<[u8; 20]> {
    const AUTHOR_NAME: &str = "ArshiAAkhavan <letmemakenewone@gmail.com>";
    const COMMITER_NAME: &str = AUTHOR_NAME;

    let start = SystemTime::now();
    let time_millis = start
        .duration_since(UNIX_EPOCH)
        .context("Time went backwards")?
        .as_millis();
    let mut content = String::new();
    writeln!(content, "tree {tree}")?;
    writeln!(content, "parent {parent}")?;
    writeln!(content, "author {AUTHOR_NAME} {time_millis} -0500")?;
    writeln!(content, "committer {COMMITER_NAME} {time_millis} -0500")?;
    writeln!(content, "\n{message}")?;

    let commit = git::Object::new(git::Kind::Commit, content.as_bytes().to_owned());
    commit.persist()?;

    Ok(commit.hash())
}

fn display_hex(hash: &[u8]) -> String {
    hash.iter()
        .fold(String::new(), |i, b| format!("{i}{b:02x}"))
}
