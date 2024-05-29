use {super::*, http::header::HeaderValue, io::Cursor, std::str};

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize, Eq, Default)]
pub struct Inscription {
    pub body: Option<Vec<u8>>,
    pub content_encoding: Option<Vec<u8>>,
    pub content_type: Option<Vec<u8>>,
    pub delegate: Option<Vec<u8>>,
    pub duplicate_field: bool,
    pub incomplete_field: bool,
    pub metadata: Option<Vec<u8>>,
    pub metaprotocol: Option<Vec<u8>>,
    pub parent: Option<Vec<u8>>,
    pub pointer: Option<Vec<u8>>,
    pub unrecognized_even_field: bool,
}

impl Inscription {
    fn inscription_id_field(field: &Option<Vec<u8>>) -> Option<InscriptionId> {
        let value = field.as_ref()?;

        if value.len() < Txid::LEN {
            return None;
        }

        if value.len() > Txid::LEN + 4 {
            return None;
        }

        let (txid, index) = value.split_at(Txid::LEN);

        if let Some(last) = index.last() {
            // Accept fixed length encoding with 4 bytes (with potential trailing zeroes)
            // or variable length (no trailing zeroes)
            if index.len() != 4 && *last == 0 {
                return None;
            }
        }

        let txid = Txid::from_slice(txid).unwrap();

        let index = [
            index.first().copied().unwrap_or(0),
            index.get(1).copied().unwrap_or(0),
            index.get(2).copied().unwrap_or(0),
            index.get(3).copied().unwrap_or(0),
        ];

        let index = u32::from_le_bytes(index);

        Some(InscriptionId { txid, index })
    }

    pub(crate) fn media(&self) -> Media {
        if self.body.is_none() {
            return Media::Unknown;
        }

        let Some(content_type) = self.content_type() else {
      return Media::Unknown;
    };

        content_type.parse().unwrap_or(Media::Unknown)
    }

    pub(crate) fn body(&self) -> Option<&[u8]> {
        Some(self.body.as_ref()?)
    }

    pub(crate) fn into_body(self) -> Option<Vec<u8>> {
        self.body
    }

    pub(crate) fn content_length(&self) -> Option<usize> {
        Some(self.body()?.len())
    }

    pub(crate) fn content_type(&self) -> Option<&str> {
        str::from_utf8(self.content_type.as_ref()?).ok()
    }

    pub(crate) fn content_encoding(&self) -> Option<HeaderValue> {
        HeaderValue::from_str(str::from_utf8(self.content_encoding.as_ref()?).unwrap_or_default())
            .ok()
    }

    pub(crate) fn delegate(&self) -> Option<InscriptionId> {
        Self::inscription_id_field(&self.delegate)
    }

    pub(crate) fn metadata(&self) -> Option<Value> {
        ciborium::from_reader(Cursor::new(self.metadata.as_ref()?)).ok()
    }

    pub(crate) fn metaprotocol(&self) -> Option<&str> {
        str::from_utf8(self.metaprotocol.as_ref()?).ok()
    }

    pub(crate) fn parent(&self) -> Option<InscriptionId> {
        Self::inscription_id_field(&self.parent)
    }

    pub(crate) fn pointer(&self) -> Option<u64> {
        let value = self.pointer.as_ref()?;

        if value.iter().skip(8).copied().any(|byte| byte != 0) {
            return None;
        }

        let pointer = [
            value.first().copied().unwrap_or(0),
            value.get(1).copied().unwrap_or(0),
            value.get(2).copied().unwrap_or(0),
            value.get(3).copied().unwrap_or(0),
            value.get(4).copied().unwrap_or(0),
            value.get(5).copied().unwrap_or(0),
            value.get(6).copied().unwrap_or(0),
            value.get(7).copied().unwrap_or(0),
        ];

        Some(u64::from_le_bytes(pointer))
    }

    pub(crate) fn hidden(&self) -> bool {
        use regex::bytes::Regex;

        const BVM_NETWORK: &[u8] = b"<body style=\"background:#F61;color:#fff;\">\
                        <h1 style=\"height:100%\">bvm.network</h1></body>";

        lazy_static! {
            static ref BRC_420: Regex =
                Regex::new(r"^\s*/content/[[:xdigit:]]{64}i\d+\s*$").unwrap();
        }

        self.body()
            .map(|body| BRC_420.is_match(body) || body.starts_with(BVM_NETWORK))
            .unwrap_or_default()
            || self.metaprotocol.is_some()
            || matches!(self.media(), Media::Code(_) | Media::Text | Media::Unknown)
    }
}
