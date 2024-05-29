use super::*;

pub(crate) struct Iframe {
  inscription_id: InscriptionId,
  thumbnail: bool,
}

impl Iframe {
  pub(crate) fn thumbnail(inscription_id: InscriptionId) -> Trusted<Self> {
    Trusted(Self {
      inscription_id,
      thumbnail: true,
    })
  }

  pub(crate) fn main(inscription_id: InscriptionId) -> Trusted<Self> {
    Trusted(Self {
      inscription_id,
      thumbnail: false,
    })
  }
}

impl Display for Iframe {
  fn fmt(&self, f: &mut Formatter) -> fmt::Result {
    if self.thumbnail {
      write!(f, "<a href=/inscription/{}>", self.inscription_id)?;
    }

    write!(
      f,
      "<iframe sandbox=allow-scripts scrolling=no loading=lazy src=/preview/{}></iframe>",
      self.inscription_id
    )?;

    if self.thumbnail {
      write!(f, "</a>",)?
    }

    Ok(())
  }
}

