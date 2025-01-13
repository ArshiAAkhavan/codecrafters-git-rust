use reqwest;
use std::env::current_dir;
use std::fmt::Write;
use std::fs;
use std::io::BufRead;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
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
    Clone {
        url: String,
        directory: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = GitCli::parse();
    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    match cli.cmd {
        GitCmd::Init => {
            init(&PathBuf::from("."))?;
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
        GitCmd::Clone { url, directory } => {
            let dst = &PathBuf::from(directory);
            std::fs::create_dir_all(dst)?;
            init(dst)?;
            git::git_clone(&url, dst)?;
            //git_clone_dumb(&url, &directory)?;
        }
    }
    Ok(())
}

fn init(current_dir: &PathBuf) -> anyhow::Result<()> {
    fs::create_dir(current_dir.join(".git")).context("failed to create the git directory")?;
    fs::create_dir(current_dir.join(".git/objects"))
        .context("failed to create the objects database")?;
    fs::create_dir(current_dir.join(".git/refs")).context("failed to create the refs")?;
    fs::write(current_dir.join(".git/HEAD"), "ref: refs/heads/master\n")
        .context("failed to specify the HEAD")?;
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
    let tree = git::Tree::try_from(obj)?;
    if name_only {
        for node in tree.nodes {
            println!("{}", node.name);
        }
    } else {
        print!("{tree:?}");
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
    let tree = git::Object::new(git::ObjectKind::Tree, buf);
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

    let commit = git::Object::new(git::ObjectKind::Commit, content.as_bytes().to_owned());
    commit.persist()?;

    Ok(commit.hash())
}
fn git_clone_dumb(url: &str, directory: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(directory).context("failed to create the target directory")?;

    let body = reqwest::blocking::get(format!("{url}/info/refs"))?;
    let body = body.text()?;
    let head = body
        .lines()
        .last()
        .ok_or(anyhow!("malformed response for info/refs"))?;
    let Some((hash, head_ref)) = head.split_once(char::is_whitespace) else {
        anyhow::bail!("failed to extract HEAD commit")
    };
    let current_dir = PathBuf::from(directory);
    init(&current_dir)?;
    fs::write(".git/HEAD", format!("ref: {head_ref}\n")).context("failed to specify the HEAD")?;

    fetch_commit(url, hash, &current_dir)
}
fn fetch_commit(url: &str, hash: &str, current_dir: &PathBuf) -> anyhow::Result<()> {
    let obj = git::Object::fetch(url, hash)?;
    obj.persist()?;
    for line in obj.body.lines() {
        let line = line?;
        let Some((obj_type, hash)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        match obj_type {
            "tree" => {
                fetch_tree(url, hash, &current_dir)?;
            }
            "parent" => {
                fetch_commit(url, hash, &current_dir)?;
            }
            _ => (),
        }
    }

    Ok(())
}

fn fetch_tree(url: &str, hash: &str, current_dir: &PathBuf) -> anyhow::Result<()> {
    eprintln!("fetching tree: {hash}");

    let obj = git::Object::fetch(url, hash)?;
    obj.persist()?;
    let tree = git::Tree::try_from(obj)?;
    for node in tree.nodes {
        match node.kind {
            git::NodeKind::Dir { .. } => {
                let dir_path = current_dir.join(&node.name);
                std::fs::create_dir_all(&dir_path).context(format!(
                    "failed to create a directory for tree {}",
                    node.name
                ))?;
                fetch_tree(url, &display_hex(&node.hash), &dir_path)?;
            }
            git::NodeKind::File { .. } | git::NodeKind::SymLink { .. } => {
                fetch_file(url, &node, current_dir.clone())?;
            }
        }
    }
    Ok(())
}

fn fetch_file(url: &str, node: &git::Node, current_dir: PathBuf) -> anyhow::Result<()> {
    let file_path = current_dir.join(&node.name);
    if file_path.exists() {
        return Ok(());
    }
    eprintln!("fetching file: {} [{}]", node.name, display_hex(&node.hash));

    let hash = display_hex(&node.hash);
    let obj = git::Object::fetch(url, &hash)?;
    obj.persist()?;

    // create file with correct permissions
    std::fs::File::create(&file_path)?;
    let permissions = PermissionsExt::from_mode(node.kind.mode() % (1 << 9));
    std::fs::set_permissions(&file_path, permissions)?;

    std::fs::write(&file_path, obj.body)?;
    Ok(())
}

fn display_hex(hash: &[u8]) -> String {
    hash.iter()
        .fold(String::new(), |i, b| format!("{i}{b:02x}"))
}
