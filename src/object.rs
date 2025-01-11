use anyhow::{anyhow, Context, Ok};
use sha1::Digest;
use std::{
    fmt::Display,
    io::Read,
    path::{Path, PathBuf},
    str,
};

pub struct Object {
    kind: ObjectKind,
    pub body: Vec<u8>,
}

impl Object {
    pub fn hash(&self) -> [u8; 20] {
        let mut hasher = sha1::Sha1::new();
        hasher.update(format!("{} {}\0", self.kind, self.body.len()));
        hasher.update(&self.body);
        let sha1sum = hasher.finalize();
        sha1sum.into()
    }
    pub fn persist(&self) -> anyhow::Result<[u8; 20]> {
        let hash = self.hash();
        let path = Object::path(&hash);
        Object::ensure_dir(
            path.parent()
                .ok_or(anyhow!("failed to ensure parent directory for object"))?,
        )?;
        let mut raw = format!("{} {}\0", self.kind, self.body.len())
            .as_bytes()
            .to_vec();
        raw.extend(&self.body);

        let mut zlib_encoder =
            flate2::read::ZlibEncoder::new(&raw[..], flate2::Compression::none());
        let mut buf = Vec::new();
        zlib_encoder.read_to_end(&mut buf)?;
        std::fs::write(path, buf)?;
        Ok(hash)
    }

    pub fn load(hex: &str) -> anyhow::Result<Self> {
        let object = std::fs::File::open(Object::path_from_hex(hex))
            .context("failed to find the object file")?;
        Object::new_object_from(object)
    }

    fn new_object_from<R: Read>(raw: R) -> anyhow::Result<Self> {
        let mut zlib_decoder = flate2::read::ZlibDecoder::new(raw);

        let mut buf = Vec::new();
        zlib_decoder.read_to_end(&mut buf)?;
        let space_position = buf
            .iter()
            .position(|c| *c == b' ')
            .ok_or(anyhow!("malformed object"))?;
        let kind = ObjectKind::try_from(&buf[..space_position])?;

        let body_position = buf
            .iter()
            .position(|c| *c == b'\0')
            .ok_or(anyhow!("malformed object"))?;

        Ok(Self {
            kind,
            body: buf[body_position + 1..].to_owned(),
        })
    }

    pub fn fetch(url: &str, hex: &str) -> anyhow::Result<Self> {
        let body = reqwest::blocking::get(format!("{url}/objects/{}/{}", &hex[..2], &hex[2..]))?;
        Object::new_object_from(std::io::Cursor::new(body.bytes()?.to_vec()))
    }

    fn ensure_dir(path: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(path)
            .context(format!("failed to create object directory for {path:?}"))
    }
    fn path_from_hex(hex: &str) -> PathBuf {
        let mut p = PathBuf::from(".git/objects");
        p.push(&hex[..2]);
        p.push(&hex[2..]);
        p
    }
    fn path(hash: &[u8]) -> PathBuf {
        let hex = hash
            .iter()
            .fold(String::new(), |i, b| format!("{i}{b:02x}"));
        Self::path_from_hex(&hex)
    }
}

impl Object {
    pub fn new_blob_from_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let mut object = std::fs::File::open(path).context("failed to open the file to hash")?;

        let mut buf = Vec::new();
        let _ = object
            .read_to_end(&mut buf)
            .context("failed to read from the file")?;
        Ok(Self {
            kind: ObjectKind::Blob,
            body: buf,
        })
    }
    pub fn new(kind: ObjectKind, body: Vec<u8>) -> Self {
        Self { kind, body }
    }
}

pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
}

impl Display for ObjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let display = match self {
            ObjectKind::Blob => "blob",
            ObjectKind::Tree => "tree",
            ObjectKind::Commit => "commit",
        };
        write!(f, "{display}")
    }
}

impl TryFrom<&[u8]> for ObjectKind {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match str::from_utf8(value)? {
            "blob" => Ok(Self::Blob),
            "tree" => Ok(Self::Tree),
            "commit" => Ok(Self::Commit),
            kind => anyhow::bail!("unknown object format! {kind}"),
        }
    }
}

#[derive(Debug)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl TryFrom<Object> for Tree {
    type Error = anyhow::Error;

    fn try_from(value: Object) -> Result<Self, Self::Error> {
        let obj = value;
        let mut nodes = Vec::new();
        let mut ptr = 0;
        while let Some(mode_end_index) = obj.body[ptr..].iter().position(|c| *c == b' ') {
            let mode = str::from_utf8(&obj.body[ptr..ptr + mode_end_index])?;
            ptr += mode_end_index + 1;

            if let Some(name_end_index) = obj.body[ptr..].iter().position(|c| *c == b'\0') {
                let name = str::from_utf8(&obj.body[ptr..ptr + name_end_index])?.into();
                ptr += name_end_index + 1;
                let mut hash = [0u8; 20];
                hash.copy_from_slice(&obj.body[ptr..ptr + 20]);
                ptr += 20;
                let kind = match mode {
                    "40000" => NodeKind::Dir { mode: 0o40000 },
                    "120000" => NodeKind::SymLink { mode: 0o120000 },
                    "100644" => NodeKind::File { mode: 0o100644 },
                    "100755" => NodeKind::File { mode: 0o100755 },
                    _ => anyhow::bail!("malformed tree node"),
                };
                nodes.push(Node { kind, name, hash });
            } else {
                panic!("malformed tree");
            }
        }
        Ok(Self { nodes })
    }
}

#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub kind: NodeKind,
    pub hash: [u8; 20],
}

#[derive(Debug)]
pub enum NodeKind {
    Dir { mode: u32 },
    File { mode: u32 },
    SymLink { mode: u32 },
}

impl NodeKind {
    pub fn mode(&self) -> u32 {
        match self {
            NodeKind::Dir { mode } | NodeKind::File { mode } | NodeKind::SymLink { mode } => *mode,
        }
    }
}
