use super::*;

const MAX_SPACERS: u32 = 0b00000111_11111111_11111111_11111111;

#[derive(Default, Serialize, Debug, PartialEq)]
pub struct Runestone {
  pub burn: bool,
  pub claim: Option<u128>,
  pub default_output: Option<u32>,
  pub edicts: Vec<Edict>,
  pub etching: Option<Etching>,
}

struct Message {
  fields: HashMap<u128, u128>,
  edicts: Vec<Edict>,
}

impl Message {
  fn from_integers(payload: &[u128]) -> Self {
    let mut edicts = Vec::new();
    let mut fields = HashMap::new();

    for i in (0..payload.len()).step_by(2) {
      let tag = payload[i];

      if Tag::Body == tag {
        let mut id = 0u128;
        for chunk in payload[i + 1..].chunks_exact(3) {
          id = id.saturating_add(chunk[0]);
          edicts.push(Edict {
            id,
            amount: chunk[1],
            output: chunk[2],
          });
        }
        break;
      }

      let Some(&value) = payload.get(i + 1) else {
        break;
      };

      fields.entry(tag).or_insert(value);
    }

    Self { fields, edicts }
  }
}

impl Runestone {
  pub fn from_transaction(transaction: &Transaction) -> Option<Self> {
    Self::decipher(transaction).ok().flatten()
  }

  fn decipher(transaction: &Transaction) -> Result<Option<Self>, script::Error> {
    let Some(payload) = Runestone::payload(transaction)? else {
      return Ok(None);
    };

    let integers = Runestone::integers(&payload);

    let Message { mut fields, edicts } = Message::from_integers(&integers);

    let claim = Tag::Claim.take(&mut fields);

    let deadline = Tag::Deadline
      .take(&mut fields)
      .and_then(|deadline| u32::try_from(deadline).ok());

    let default_output = Tag::DefaultOutput
      .take(&mut fields)
      .and_then(|default| u32::try_from(default).ok());

    let divisibility = Tag::Divisibility
      .take(&mut fields)
      .and_then(|divisibility| u8::try_from(divisibility).ok())
      .and_then(|divisibility| (divisibility <= MAX_DIVISIBILITY).then_some(divisibility))
      .unwrap_or_default();

    let limit = Tag::Limit
      .take(&mut fields)
      .map(|limit| limit.min(MAX_LIMIT));

    let rune = Tag::Rune.take(&mut fields).map(Rune);

    let spacers = Tag::Spacers
      .take(&mut fields)
      .and_then(|spacers| u32::try_from(spacers).ok())
      .and_then(|spacers| (spacers <= MAX_SPACERS).then_some(spacers))
      .unwrap_or_default();

    let symbol = Tag::Symbol
      .take(&mut fields)
      .and_then(|symbol| u32::try_from(symbol).ok())
      .and_then(char::from_u32);

    let term = Tag::Term
      .take(&mut fields)
      .and_then(|term| u32::try_from(term).ok());

    let mut flags = Tag::Flags.take(&mut fields).unwrap_or_default();

    let etch = Flag::Etch.take(&mut flags);

    let mint = Flag::Mint.take(&mut flags);

    let etching = if etch {
      Some(Etching {
        divisibility,
        rune,
        spacers,
        symbol,
        mint: mint.then_some(Mint {
          deadline,
          limit,
          term,
        }),
      })
    } else {
      None
    };

    Ok(Some(Self {
      burn: flags != 0 || fields.keys().any(|tag| tag % 2 == 0),
      claim,
      default_output,
      edicts,
      etching,
    }))
  }

  pub(crate) fn encipher(&self) -> ScriptBuf {
    let mut payload = Vec::new();

    if let Some(etching) = self.etching {
      let mut flags = 0;
      Flag::Etch.set(&mut flags);

      if etching.mint.is_some() {
        Flag::Mint.set(&mut flags);
      }

      Tag::Flags.encode(flags, &mut payload);

      if let Some(rune) = etching.rune {
        Tag::Rune.encode(rune.0, &mut payload);
      }

      if etching.divisibility != 0 {
        Tag::Divisibility.encode(etching.divisibility.into(), &mut payload);
      }

      if etching.spacers != 0 {
        Tag::Spacers.encode(etching.spacers.into(), &mut payload);
      }

      if let Some(symbol) = etching.symbol {
        Tag::Symbol.encode(symbol.into(), &mut payload);
      }

      if let Some(mint) = etching.mint {
        if let Some(deadline) = mint.deadline {
          Tag::Deadline.encode(deadline.into(), &mut payload);
        }

        if let Some(limit) = mint.limit {
          Tag::Limit.encode(limit, &mut payload);
        }

        if let Some(term) = mint.term {
          Tag::Term.encode(term.into(), &mut payload);
        }
      }
    }

    if let Some(claim) = self.claim {
      Tag::Claim.encode(claim, &mut payload);
    }

    if let Some(default_output) = self.default_output {
      Tag::DefaultOutput.encode(default_output.into(), &mut payload);
    }

    if self.burn {
      Tag::Burn.encode(0, &mut payload);
    }

    if !self.edicts.is_empty() {
      varint::encode_to_vec(Tag::Body.into(), &mut payload);

      let mut edicts = self.edicts.clone();
      edicts.sort_by_key(|edict| edict.id);

      let mut id = 0;
      for edict in edicts {
        varint::encode_to_vec(edict.id - id, &mut payload);
        varint::encode_to_vec(edict.amount, &mut payload);
        varint::encode_to_vec(edict.output, &mut payload);
        id = edict.id;
      }
    }

    let mut builder = script::Builder::new()
      .push_opcode(opcodes::all::OP_RETURN)
      .push_slice(b"RUNE_TEST");

    for chunk in payload.chunks(MAX_SCRIPT_ELEMENT_SIZE) {
      let push: &script::PushBytes = chunk.try_into().unwrap();
      builder = builder.push_slice(push);
    }

    builder.into_script()
  }

  fn payload(transaction: &Transaction) -> Result<Option<Vec<u8>>, script::Error> {
    for output in &transaction.output {
      let mut instructions = output.script_pubkey.instructions();

      if instructions.next().transpose()? != Some(Instruction::Op(opcodes::all::OP_RETURN)) {
        continue;
      }

      if instructions.next().transpose()? != Some(Instruction::PushBytes(b"RUNE_TEST".into())) {
        continue;
      }

      let mut payload = Vec::new();

      for result in instructions {
        if let Instruction::PushBytes(push) = result? {
          payload.extend_from_slice(push.as_bytes());
        }
      }

      return Ok(Some(payload));
    }

    Ok(None)
  }

  fn integers(payload: &[u8]) -> Vec<u128> {
    let mut integers = Vec::new();
    let mut i = 0;

    while i < payload.len() {
      let (integer, length) = varint::decode(&payload[i..]);
      integers.push(integer);
      i += length;
    }

    integers
  }
}
