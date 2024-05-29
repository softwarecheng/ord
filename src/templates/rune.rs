use super::*;

#[derive(Boilerplate, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuneHtml {
  pub entry: RuneEntry,
  pub id: RuneId,
  pub parent: Option<InscriptionId>,
}

impl PageContent for RuneHtml {
  fn title(&self) -> String {
    format!("Rune {}", self.entry.spaced_rune())
  }
}
