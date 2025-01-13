use anyhow::{anyhow, Context};
use reqwest;

use std::collections::HashMap;
use std::io::BufRead;
use std::io::Read;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::{str, usize};

use crate::object::Object;
use crate::ObjectKind;

#[derive(Debug)]
struct Packet {
    objects: HashMap<[u8; 20], Object>,
}

#[derive(Debug)]
enum ObjectType {
    Commit = 1,
    Tree = 2,
    Blob = 3,
    Tag = 4,
    OfsDelta = 6,
    RefDelta = 7,
}

impl TryFrom<u8> for ObjectType {
    type Error = anyhow::Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(ObjectType::Commit),
            2 => Ok(ObjectType::Tree),
            3 => Ok(ObjectType::Blob),
            4 => Ok(ObjectType::Tag),
            6 => Ok(ObjectType::OfsDelta),
            7 => Ok(ObjectType::RefDelta),
            _ => anyhow::bail!("unknown object type {value}"),
        }
    }
}

impl TryFrom<bytes::Bytes> for Packet {
    type Error = anyhow::Error;

    fn try_from(raw: bytes::Bytes) -> Result<Self, Self::Error> {
        let pos = raw.iter().position(|c| *c == b'\n').unwrap_or_default() as usize;
        let original_raw = &raw[..raw.len() - 20];
        let raw = &raw[pos + 1..];
        let magic_prefix = &raw[..4];
        assert_eq!(magic_prefix, b"PACK");

        let _version = &raw[4..8];
        let num_objects = u32::from_be_bytes(raw[8..12].try_into()?) as usize;

        let mut packet = Packet {
            objects: HashMap::with_capacity(num_objects),
        };

        let raw = original_raw;
        let mut ptr = 12 + pos + 1;
        while ptr < raw.len() {
            let obj_type_byte = raw[ptr];
            let obj_type = ObjectType::try_from((obj_type_byte & 0b0111_0000) >> 4)?;
            let mut obj_len_byte = raw[ptr];
            let mut obj_len = (obj_len_byte & 0b1111) as usize;
            let mut shift_count = 4;
            while obj_len_byte & 0b1000_0000 != 0 {
                ptr += 1;
                obj_len_byte = raw[ptr];
                obj_len = obj_len + (((obj_len_byte & 0b0111_1111) as usize) << shift_count);
                shift_count += 8;
            }
            ptr += 1;

            let (obj, nbytes) = match obj_type {
                ObjectType::OfsDelta => unimplemented!(),
                ObjectType::RefDelta => calculate_delta(&raw[ptr..], obj_len, &packet)?,
                ObjectType::Commit | ObjectType::Tree | ObjectType::Blob | ObjectType::Tag => {
                    let mut buf = Vec::new();

                    let mut cursor = std::io::Cursor::new(&raw[ptr..]);
                    let mut zlib_decoder = flate2::bufread::ZlibDecoder::new(&mut cursor);
                    zlib_decoder.read_to_end(&mut buf)?;

                    assert_eq!(buf.len(), obj_len);

                    let nbytes = cursor.position() as usize;
                    (
                        crate::Object {
                            kind: crate::ObjectKind::try_from(obj_type)?,
                            body: buf,
                        },
                        nbytes,
                    )
                }
            };
            eprintln!("unpacked {}:\t{}", obj.kind, (display_hex(&obj.hash())));
            packet.objects.insert(obj.hash(), obj);
            ptr += nbytes;
        }
        Ok(packet)
    }
}

fn calculate_delta(raw: &[u8], obj_len: usize, packet: &Packet) -> anyhow::Result<(Object, usize)> {
    let base_hash = &raw[0..20];

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&raw[20..]);
    let mut zlib_decoder = flate2::bufread::ZlibDecoder::new(&mut cursor);
    zlib_decoder.read_to_end(&mut buf)?;

    assert_eq!(obj_len, buf.len());
    let nbytes = cursor.position() as usize;

    let raw = &buf[..];
    let mut ptr = 0;

    // skip the source-size bytes
    while raw[ptr] & 0b1000_0000 != 0 {
        ptr += 1;
    }
    ptr += 1;

    // skip the target-size bytes
    while raw[ptr] & 0b1000_0000 != 0 {
        ptr += 1;
    }
    ptr += 1;

    let base_object = packet
        .objects
        .get(base_hash)
        .ok_or(anyhow!("failed to find object {}", display_hex(base_hash)))?;

    let mut obj_raw = Vec::new();
    while ptr < raw.len() {
        let instruction = raw[ptr];
        ptr += 1;
        let instruction_opcode = instruction & 0b1000_0000;
        match instruction_opcode != 0 {
            // copy instruction
            true => {
                let mut ofset_opcode = instruction % 0b1_0000;
                let mut ofset = 0usize;
                let mut shift_amount = 0;
                for _ in 0..4 {
                    let should_copy = ofset_opcode % 2;
                    let ofset_byte = if should_copy == 1 {
                        ptr += 1;
                        raw[ptr - 1]
                    } else {
                        0
                    };
                    ofset = ofset + ((ofset_byte as usize) << shift_amount);
                    shift_amount += 8;
                    ofset_opcode >>= 1;
                }
                let mut len_opcode = (instruction >> 4) % 0b1000;
                let mut len = 0;
                let mut shift_amount = 0;
                for _ in 0..3 {
                    let should_copy = len_opcode % 2;
                    let len_byte = if should_copy == 1 {
                        ptr += 1;
                        raw[ptr - 1]
                    } else {
                        0
                    };
                    len = len + ((len_byte as usize) << shift_amount);
                    shift_amount += 8;
                    len_opcode >>= 1;
                }
                obj_raw.extend(&base_object.body[ofset..ofset + len])
            }
            // insert instruction
            false => {
                let nbytes = instruction as usize;
                obj_raw.extend(&raw[ptr..ptr + nbytes]);
                ptr += nbytes;
            }
        }
    }
    let obj = crate::Object {
        kind: crate::ObjectKind::Blob,
        body: obj_raw,
    };

    Ok((obj, nbytes + 20))
}

#[derive(Debug)]
struct PacketLine {
    data: Vec<u8>,
}

impl PacketLine {
    fn len(&self) -> usize {
        self.data.len()
    }
}

struct PacketLineBuilder {
    wants: Vec<String>,
}
impl PacketLineBuilder {
    fn new() -> Self {
        Self { wants: Vec::new() }
    }

    fn want(&mut self, hex: String) {
        self.wants.push(hex)
    }

    fn build(self) -> PacketLine {
        let mut data = Vec::new();
        for hex in self.wants {
            let _ = writeln!(data, "{:04x}want {hex}", 4 + 5 + hex.len() + 1);
        }
        let _ = write!(data, "0000");
        let _ = writeln!(data, "0009done");
        PacketLine { data }
    }
}

impl<'a> TryFrom<&'a [u8]> for PacketLine {
    type Error = anyhow::Error;

    fn try_from(value: &'a [u8]) -> Result<Self, Self::Error> {
        if value.len() < 4 {
            anyhow::bail!("packet line size can't be less than four")
        }
        let len: u32 = u32::from_str_radix(str::from_utf8(&value[..4])?, 16)?;
        let len = len as usize;
        if len + 4 > value.len() {
            anyhow::bail!("packet line size greater than the byte stream")
        }
        match len {
            0 => Ok(Self { data: Vec::new() }),
            _ => Ok(Self {
                data: value[4..len].to_vec(),
            }),
        }
    }
}

struct PacketLineIterator {
    stream: Vec<u8>,
}

impl Iterator for PacketLineIterator {
    type Item = PacketLine;

    fn next(&mut self) -> Option<Self::Item> {
        let next_packet = PacketLine::try_from(self.stream.as_slice()).ok()?;
        self.stream.drain(..next_packet.len() + 4);
        Some(next_packet)
    }
}

trait IntoPackeLineIterator {
    fn into_packet_line_iter(self) -> PacketLineIterator;
}

impl IntoPackeLineIterator for bytes::Bytes {
    fn into_packet_line_iter(self) -> PacketLineIterator {
        PacketLineIterator {
            stream: self.to_vec(),
        }
    }
}

pub fn git_clone(url: &str, dst: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dst)?;
    let client = reqwest::blocking::Client::new();
    let refs = do_info_refs_request(&client, url)?;
    dbg!(&refs);
    let head_hash = refs
        .iter()
        .find(|(name, _)| name == "HEAD")
        .map(|(_, hash)| hash)
        .take()
        .ok_or(anyhow!("no HEADS in refs"))?;
    let packet = get_objects_of_refs(&client, url, refs.clone())?;
    for obj in packet.objects.values() {
        obj.persist()?;
    }
    rebuild_directory_from_head(head_hash, &dst)?;

    Ok(())
}

fn get_objects_of_refs(
    client: &reqwest::blocking::Client,
    url: &str,
    refs: Vec<(String, String)>,
) -> anyhow::Result<Packet> {
    let mut plb = PacketLineBuilder::new();
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
        .body(payload.data) // Send the binary data
        .send()?;

    //let mut x = response.bytes()?.to_vec();
    //x.push(0);
    //x.push(0);
    //x.push(0);
    //x.push(0);
    //Packet::try_from(Bytes::from(x))
    Packet::try_from(response.bytes()?)
}

fn do_info_refs_request(
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
        .skip_while(|p| p.len() != 0)
        .skip(1)
        .take_while(|p| p.len() != 0)
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

fn display_hex(hash: &[u8]) -> String {
    hash.iter()
        .fold(String::new(), |i, b| format!("{i}{b:02x}"))
}

fn rebuild_directory_from_head(head_hash: &str, current_dir: &PathBuf) -> anyhow::Result<()> {
    rebuild_commit(head_hash, current_dir)
}

fn rebuild_commit(hash: &str, current_dir: &PathBuf) -> anyhow::Result<()> {
    let obj = crate::Object::load(hash)?;
    obj.persist()?;
    for line in obj.body.lines() {
        let line = line?;
        let Some((obj_type, hash)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        match obj_type {
            "tree" => {
                rebuild_tree(hash, &current_dir)?;
            }
            "parent" => {
                rebuild_commit(hash, &current_dir)?;
            }
            _ => (),
        }
    }

    Ok(())
}

fn rebuild_tree(hash: &str, current_dir: &PathBuf) -> anyhow::Result<()> {
    eprintln!("fetching tree: {hash}");

    let obj = crate::Object::load(hash)?;
    let tree = crate::Tree::try_from(obj)?;
    for node in tree.nodes {
        match node.kind {
            crate::NodeKind::Dir { .. } => {
                let dir_path = current_dir.join(&node.name);
                std::fs::create_dir_all(&dir_path).context(format!(
                    "failed to create a directory for tree {}",
                    node.name
                ))?;
                rebuild_tree(&display_hex(&node.hash), &dir_path)?;
            }
            crate::NodeKind::File { .. } | crate::NodeKind::SymLink { .. } => {
                rebuild_file(&node, current_dir.clone())?;
            }
        }
    }
    Ok(())
}

fn rebuild_file(node: &crate::Node, current_dir: PathBuf) -> anyhow::Result<()> {
    let file_path = current_dir.join(&node.name);
    if file_path.exists() {
        return Ok(());
    }
    eprintln!("fetching file: {} [{}]", node.name, display_hex(&node.hash));

    let hash = display_hex(&node.hash);
    let obj = crate::Object::load(&hash)?;

    // create file with correct permissions
    std::fs::File::create(&file_path)?;
    let permissions = PermissionsExt::from_mode(node.kind.mode() % (1 << 9));
    std::fs::set_permissions(&file_path, permissions)?;

    std::fs::write(&file_path, obj.body)?;
    Ok(())
}

impl TryFrom<ObjectType> for ObjectKind {
    type Error = anyhow::Error;

    fn try_from(value: ObjectType) -> Result<Self, Self::Error> {
        Ok(match value {
            ObjectType::Commit => Self::Commit,
            ObjectType::Tree => Self::Tree,
            ObjectType::Blob => Self::Blob,
            ObjectType::Tag => Self::Commit,
            ObjectType::RefDelta | ObjectType::OfsDelta => anyhow::bail!("not an ObjectKind"),
        })
    }
}
