use {super::*, axum::extract::FromRef};

#[derive(Default, Debug)]
pub(crate) struct AcceptEncoding(pub(crate) Option<String>);

#[async_trait::async_trait]
impl<S> axum::extract::FromRequestParts<S> for AcceptEncoding
where
  Arc<ServerConfig>: FromRef<S>,
  S: Send + Sync,
{
  type Rejection = (StatusCode, &'static str);

  async fn from_request_parts(
    parts: &mut http::request::Parts,
    _state: &S,
  ) -> Result<Self, Self::Rejection> {
    Ok(Self(
      parts
        .headers
        .get("accept-encoding")
        .map(|value| value.to_str().unwrap_or_default().to_owned()),
    ))
  }
}

impl AcceptEncoding {
  pub(crate) fn is_acceptable(&self, encoding: &HeaderValue) -> bool {
    let Ok(encoding) = encoding.to_str() else {
      return false;
    };

    self
      .0
      .clone()
      .unwrap_or_default()
      .split(',')
      .any(|value| value.split(';').next().unwrap_or_default().trim() == encoding)
  }
}
