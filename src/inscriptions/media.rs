use {
    self::{ImageRendering::*, Language::*, Media::*},
    super::*,
    brotli::enc::backward_references::BrotliEncoderMode::{
        self, BROTLI_MODE_FONT as FONT, BROTLI_MODE_GENERIC as GENERIC, BROTLI_MODE_TEXT as TEXT,
    },
};

#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum Media {
    Audio,
    Code(Language),
    Font,
    Iframe,
    Image(ImageRendering),
    Markdown,
    Model,
    Pdf,
    Text,
    Unknown,
    Video,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum Language {
    Css,
    JavaScript,
    Json,
    Python,
    Yaml,
}

impl Display for Language {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Css => "css",
                Self::JavaScript => "javascript",
                Self::Json => "json",
                Self::Python => "python",
                Self::Yaml => "yaml",
            }
        )
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub(crate) enum ImageRendering {
    Auto,
    Pixelated,
}

impl Display for ImageRendering {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Auto => "auto",
                Self::Pixelated => "pixelated",
            }
        )
    }
}

impl Media {
    #[rustfmt::skip]
    const TABLE: &'static [(&'static str, BrotliEncoderMode, Media, &'static [&'static str])] = &[
    ("application/cbor",            GENERIC, Unknown,          &["cbor"]),
    ("application/json",            TEXT,    Code(Json),       &["json"]),
    ("application/octet-stream",    GENERIC, Unknown,          &["bin"]),
    ("application/pdf",             GENERIC, Pdf,              &["pdf"]),
    ("application/pgp-signature",   TEXT,    Text,             &["asc"]),
    ("application/protobuf",        GENERIC, Unknown,          &["binpb"]),
    ("application/x-javascript",    TEXT,    Code(JavaScript), &[]),
    ("application/yaml",            TEXT,    Code(Yaml),       &["yaml", "yml"]),
    ("audio/flac",                  GENERIC, Audio,            &["flac"]),
    ("audio/mpeg",                  GENERIC, Audio,            &["mp3"]),
    ("audio/wav",                   GENERIC, Audio,            &["wav"]),
    ("font/otf",                    GENERIC, Font,             &["otf"]),
    ("font/ttf",                    GENERIC, Font,             &["ttf"]),
    ("font/woff",                   GENERIC, Font,             &["woff"]),
    ("font/woff2",                  FONT,    Font,             &["woff2"]),
    ("image/apng",                  GENERIC, Image(Pixelated), &["apng"]),
    ("image/avif",                  GENERIC, Image(Auto),      &["avif"]),
    ("image/gif",                   GENERIC, Image(Pixelated), &["gif"]),
    ("image/jpeg",                  GENERIC, Image(Pixelated), &["jpg", "jpeg"]),
    ("image/jxl",                   GENERIC, Image(Auto),      &[]),
    ("image/png",                   GENERIC, Image(Pixelated), &["png"]),
    ("image/svg+xml",               TEXT,    Iframe,           &["svg"]),
    ("image/webp",                  GENERIC, Image(Pixelated), &["webp"]),
    ("model/gltf+json",             TEXT,    Model,            &["gltf"]),
    ("model/gltf-binary",           GENERIC, Model,            &["glb"]),
    ("model/stl",                   GENERIC, Unknown,          &["stl"]),
    ("text/css",                    TEXT,    Code(Css),        &["css"]),
    ("text/html",                   TEXT,    Iframe,           &[]),
    ("text/html;charset=utf-8",     TEXT,    Iframe,           &["html"]),
    ("text/javascript",             TEXT,    Code(JavaScript), &["js"]),
    ("text/markdown",               TEXT,    Markdown,         &[]),
    ("text/markdown;charset=utf-8", TEXT,    Markdown,         &["md"]),
    ("text/plain",                  TEXT,    Text,             &[]),
    ("text/plain;charset=utf-8",    TEXT,    Text,             &["txt"]),
    ("text/x-python",               TEXT,    Code(Python),     &["py"]),
    ("video/mp4",                   GENERIC, Video,            &["mp4"]),
    ("video/webm",                  GENERIC, Video,            &["webm"]),
  ];
}

impl FromStr for Media {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        for entry in Self::TABLE {
            if entry.0 == s {
                return Ok(entry.2);
            }
        }

        Err(anyhow!("unknown content type: {s}"))
    }
}
