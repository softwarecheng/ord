use super::*;

#[derive(Boilerplate)]
pub(crate) struct OutputHtml {
  pub(crate) chain: Chain,
  pub(crate) inscriptions: Vec<InscriptionId>,
  pub(crate) outpoint: OutPoint,
  pub(crate) output: TxOut,
  pub(crate) runes: Vec<(SpacedRune, Pile)>,
  pub(crate) sat_ranges: Option<Vec<(u64, u64)>>,
  pub(crate) spent: bool,
}

impl PageContent for OutputHtml {
  fn title(&self) -> String {
    format!("Output {}", self.outpoint)
  }
}

