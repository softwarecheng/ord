use super::*;

#[derive(Boilerplate)]
pub(crate) struct InscriptionsBlockHtml {
  pub(crate) block: u32,
  pub(crate) inscriptions: Vec<InscriptionId>,
  pub(crate) prev_block: Option<u32>,
  pub(crate) next_block: Option<u32>,
  pub(crate) prev_page: Option<u32>,
  pub(crate) next_page: Option<u32>,
}

impl InscriptionsBlockHtml {
  pub(crate) fn new(
    block: u32,
    current_blockheight: u32,
    inscriptions: Vec<InscriptionId>,
    more_inscriptions: bool,
    page_index: u32,
  ) -> Result<Self> {
    if inscriptions.is_empty() {
      return Err(anyhow!("page index {page_index} exceeds inscription count"));
    }

    Ok(Self {
      block,
      inscriptions,
      prev_block: block.checked_sub(1),
      next_block: if current_blockheight > block {
        Some(block + 1)
      } else {
        None
      },
      prev_page: page_index.checked_sub(1),
      next_page: if more_inscriptions {
        Some(page_index + 1)
      } else {
        None
      },
    })
  }
}

impl PageContent for InscriptionsBlockHtml {
  fn title(&self) -> String {
    format!("Inscriptions in Block {0}", self.block)
  }
}
