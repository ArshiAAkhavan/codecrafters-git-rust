use anyhow::{anyhow, Context, Ok};
use sha1::Digest;
use std::{
    fmt::Display,
    io::Read,
    path::{Path, PathBuf},
};

pub struct Object {
    kind: Kind,
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
        Object::ensure_dir(path.parent().ok_or(anyhow!("internal error"))?)?;
        let mut raw = format!("{} {}\0", self.kind, self.body.len())
            .as_bytes()
            .to_vec();
        raw.extend(&self.body);

        let mut zlib_encoder =
            flate2::read::ZlibEncoder::new(&raw[..], flate2::Compression::none());
        let mut buf = Vec::new();
        zlib_encoder.read_to_end(&mut buf)?;
        dbg!(&path);
        std::fs::write(path, buf)?;
        dbg!("mamad");
        Ok(hash)
    }

    pub fn load(hex: &str) -> anyhow::Result<Self> {
        let object = std::fs::File::open(Object::path_from_hex(hex))
            .context("failed to find the object file")?;
        let mut zlib_decoder = flate2::read::ZlibDecoder::new(object);

        let mut buf = Vec::new();
        zlib_decoder.read_to_end(&mut buf)?;
        let space_position = buf
            .iter()
            .position(|c| *c == b' ')
            .ok_or(anyhow!("malformed object"))?;
        let kind = Kind::try_from(&buf[..space_position])?;

        let body_position = buf
            .iter()
            .position(|c| *c == b'\0')
            .ok_or(anyhow!("malformed object"))?;

        Ok(Self {
            kind,
            body: buf[body_position + 1..].to_owned(),
        })
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
            kind: Kind::Blob,
            body: buf,
        })
    }
    pub fn new(kind: Kind, body: Vec<u8>) -> Self {
        Self { kind, body }
    }
}

pub enum Kind {
    Blob,
    Tree,
    Commit,
}

impl Display for Kind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let display = match self {
            Kind::Blob => "blob",
            Kind::Tree => "tree",
            Kind::Commit => "commit",
        };
        write!(f, "{display}")
    }
}

impl TryFrom<&[u8]> for Kind {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        match &*String::from_utf8(value.to_vec())? {
            "blob" => Ok(Self::Blob),
            "tree" => Ok(Self::Tree),
            "commit" => Ok(Self::Commit),
            kind => anyhow::bail!("unknown object format! {kind}"),
        }
    }
}
