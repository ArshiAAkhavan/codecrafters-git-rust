use anyhow::{anyhow, Context};
use clap::{Parser, Subcommand};
use std::fmt::Write;
use std::fs;
use std::io::BufRead;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::str;
use std::time::{SystemTime, UNIX_EPOCH};

use codecrafters_git as git;
use git::IntoPackeLineIterator;

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
            git_clone(&url, &PathBuf::from(directory))?;
        }
    }
    Ok(())
}

fn init(current_dir: &Path) -> anyhow::Result<()> {
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

pub fn git_clone(url: &str, dst: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    init(dst)?;
    let client = reqwest::blocking::Client::new();
    let refs = fetch_refs(&client, url)?;
    dbg!(&refs);
    let head_hash = refs
        .iter()
        .find(|(name, _)| name == "HEAD")
        .map(|(_, hash)| hash)
        .take()
        .ok_or(anyhow!("no HEADs in refs"))?
        .to_owned();
    let packet = fetch_objects(&client, url, refs)?;
    build_from_head(&head_hash, dst, &packet)?;
    for obj in packet.objects.values() {
        obj.persist_in(dst)?;
    }

    Ok(())
}

fn fetch_objects(
    client: &reqwest::blocking::Client,
    url: &str,
    refs: Vec<(String, String)>,
) -> anyhow::Result<git::Packet> {
    let mut plb = git::PacketLineBuilder::new();
    for (name, hash) in refs {
        println!("{name}: {hash}");
        plb.want(hash);
    }
    let payload = plb.build();

    let response = client
        .post(format!("{url}/git-upload-pack"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-git-upload-pack-request",
        )
        .header(
            reqwest::header::ACCEPT,
            "application/x-git-upload-pack-result",
        )
        .body(payload.data)
        .send()?;

    git::Packet::try_from(response.bytes()?)
}

fn fetch_refs(
    client: &reqwest::blocking::Client,
    url: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let url = format!("{url}/info/refs");

    let response = client
        .get(url)
        .query(&[("service", "git-upload-pack")])
        .send()?;

    let body = response.bytes()?;
    let mut refs = Vec::new();
    for packet_line in body
        .into_packet_line_iter()
        .skip_while(|p| !p.is_empty())
        .skip(1)
        .take_while(|p| !p.is_empty())
    {
        let pos = packet_line
            .data
            .iter()
            .position(|c| *c == b'\0' || *c == b'\n')
            .unwrap_or(packet_line.len());
        let name = str::from_utf8(&packet_line.data[41..pos])?;
        let hash = str::from_utf8(&packet_line.data[..40])?.into();
        refs.push((name.into(), hash));
    }
    Ok(refs)
}

fn build_from_head(
    head_hash: &str,
    current_dir: &Path,
    packet: &git::Packet,
) -> anyhow::Result<()> {
    build_commit(head_hash, current_dir, packet)
}

fn build_commit(hash: &str, current_dir: &Path, packet: &git::Packet) -> anyhow::Result<()> {
    let obj = packet
        .objects
        .get(&hex_to_array(hash)?)
        .ok_or(anyhow!("failed to find {hash} in packet"))?;
    for line in obj.body.lines() {
        let line = line?;
        let Some((obj_type, hash)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        match obj_type {
            "tree" => {
                build_tree(hash, current_dir, packet)?;
            }
            "parent" => {
                build_commit(hash, current_dir, packet)?;
            }
            _ => (),
        }
    }

    Ok(())
}

fn build_tree(hash: &str, current_dir: &Path, packet: &git::Packet) -> anyhow::Result<()> {
    eprintln!("fetching tree: {hash}");

    let obj = packet
        .objects
        .get(&hex_to_array(hash)?)
        .ok_or(anyhow!("failed to find {hash} in packet"))?;
    let obj = git::Object::clone(obj);
    let tree = git::Tree::try_from(obj)?;
    for node in tree.nodes {
        match node.kind {
            git::NodeKind::Dir { .. } => {
                let dir_path = current_dir.join(&node.name);
                std::fs::create_dir_all(&dir_path).context(format!(
                    "failed to create a directory for tree {}",
                    node.name
                ))?;
                build_tree(&display_hex(&node.hash), &dir_path, packet)?;
            }
            git::NodeKind::File { .. } | git::NodeKind::SymLink { .. } => {
                build_file(&node, current_dir, packet)?;
            }
        }
    }
    Ok(())
}

fn build_file(node: &git::Node, current_dir: &Path, packet: &git::Packet) -> anyhow::Result<()> {
    let file_path = current_dir.join(&node.name);
    if file_path.exists() {
        return Ok(());
    }
    eprintln!("fetching file: {} [{}]", node.name, display_hex(&node.hash));

    let hash = display_hex(&node.hash);
    let obj = packet
        .objects
        .get(&hex_to_array(&hash)?)
        .ok_or(anyhow!("failed to find {hash} in packet"))?;

    // create file with correct permissions
    std::fs::File::create(&file_path)?;
    let permissions = PermissionsExt::from_mode(node.kind.mode() % (1 << 9));
    std::fs::set_permissions(&file_path, permissions)?;

    std::fs::write(&file_path, &obj.body)?;
    Ok(())
}

fn display_hex(hash: &[u8]) -> String {
    hash.iter()
        .fold(String::new(), |i, b| format!("{i}{b:02x}"))
}

fn hex_to_array(hex: &str) -> anyhow::Result<[u8; 20]> {
    if hex.len() != 40 {
        anyhow::bail!("Hex string must be 40 characters long.");
    }

    let mut array = [0u8; 20];
    for (i, byte) in hex.as_bytes().chunks(2).enumerate() {
        let hex_pair = std::str::from_utf8(byte)?;
        array[i] = u8::from_str_radix(hex_pair, 16)?;
    }
    Ok(array)
}
