use std::{path::PathBuf, str};

#[derive(Debug)]
struct Packet {
    hash: [u8; 20],
    body: Vec<u8>,
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
    std::fs::create_dir_all(dst)?;
    let refs = do_info_refs_request(url)?;
    for (name, hash) in refs {
        println!("{name}: {}", display_hex(&hash));
    }

    Ok(())
}

fn do_info_refs_request(url: &str) -> anyhow::Result<Vec<(String, [u8; 20])>> {
    let url = format!("{url}/info/refs");

    let client = reqwest::blocking::Client::new();

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
        let hash: Vec<u8> = packet_line.data[..40]
            .chunks(2)
            .map(|c| u8::from_str_radix(str::from_utf8(c).unwrap(), 16).unwrap())
            .collect();
        let pos = packet_line
            .data
            .iter()
            .position(|c| *c == b'\0' || *c == b'\n')
            .unwrap_or(packet_line.len());
        let name = str::from_utf8(&packet_line.data[40..pos])?;
        let hash: [u8; 20] = hash.try_into().expect("Failed to convert Vec to array");
        refs.push((name.into(), hash));
    }
    Ok(refs)
}

fn display_hex(hash: &[u8]) -> String {
    hash.iter()
        .fold(String::new(), |i, b| format!("{i}{b:02x}"))
}
