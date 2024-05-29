use super::*;

#[derive(Boilerplate, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlocksHtml {
  pub last: u32,
  pub blocks: Vec<BlockHash>,
  pub featured_blocks: BTreeMap<BlockHash, Vec<InscriptionId>>,
}

impl BlocksHtml {
  pub(crate) fn new(
    blocks: Vec<(u32, BlockHash)>,
    featured_blocks: BTreeMap<BlockHash, Vec<InscriptionId>>,
  ) -> Self {
    Self {
      last: blocks
        .first()
        .map(|(height, _)| height)
        .cloned()
        .unwrap_or(0),
      blocks: blocks.into_iter().map(|(_, hash)| hash).collect(),
      featured_blocks,
    }
  }
}

impl PageContent for BlocksHtml {
  fn title(&self) -> String {
    "Blocks".to_string()
  }
}

