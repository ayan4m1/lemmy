use crate::fetcher::post_or_comment::PostOrComment;
use anyhow::Context;
use lemmy_api_common::utils::blocking;
use lemmy_apub_lib::{
  inbox::ActorPublicKey,
  signatures::PublicKey,
  InstanceSettings,
  LocalInstance,
  DEFAULT_TIMEOUT,
};
use lemmy_db_schema::{newtypes::DbUrl, source::activity::Activity, utils::DbPool};
use lemmy_utils::{location_info, settings::structs::Settings, LemmyError};
use lemmy_websocket::LemmyContext;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Deserializer};
use std::env;
use url::{ParseError, Url};

pub mod activities;
pub(crate) mod activity_lists;
pub(crate) mod collections;
mod context;
pub mod fetcher;
pub mod http;
pub(crate) mod mentions;
pub mod objects;
pub mod protocol;

// TODO: store this in context? but its only used in this crate, no need to expose it elsewhere
fn local_instance(context: &LemmyContext) -> &'static LocalInstance {
  static LOCAL_INSTANCE: OnceCell<LocalInstance> = OnceCell::new();
  LOCAL_INSTANCE.get_or_init(|| {
    let settings = InstanceSettings::new(
      context.settings().http_fetch_retry_limit,
      context.settings().federation.worker_count,
      env::var("APUB_TESTING_SEND_SYNC").is_ok(),
      DEFAULT_TIMEOUT,
      |url| check_apub_id_valid(url, &Settings::get()),
    );
    LocalInstance::new(
      context.settings().hostname,
      context.client().clone(),
      settings,
    )
  })
}

/// Checks if the ID is allowed for sending or receiving.
///
/// In particular, it checks for:
/// - federation being enabled (if its disabled, only local URLs are allowed)
/// - the correct scheme (either http or https)
/// - URL being in the allowlist (if it is active)
/// - URL not being in the blocklist (if it is active)
///
/// `use_strict_allowlist` should be true only when parsing a remote community, or when parsing a
/// post/comment in a local community.
#[tracing::instrument(skip(settings))]
fn check_apub_id_valid(apub_id: &Url, settings: &Settings) -> Result<(), &'static str> {
  let domain = apub_id.domain().expect("apud id has domain").to_string();
  let local_instance = settings
    .get_hostname_without_port()
    .expect("local hostname is valid");
  if domain == local_instance {
    return Ok(());
  }

  if !settings.federation.enabled {
    return Err("Federation disabled");
  }

  if apub_id.scheme() != settings.get_protocol_string() {
    return Err("Invalid protocol scheme");
  }

  if let Some(blocked) = settings.to_owned().federation.blocked_instances {
    if blocked.contains(&domain) {
      return Err("Domain is blocked");
    }
  }

  if let Some(allowed) = settings.to_owned().federation.allowed_instances {
    if !allowed.contains(&domain) {
      return Err("Domain is not in allowlist");
    }
  }

  Ok(())
}

#[tracing::instrument(skip(settings))]
pub(crate) fn check_apub_id_valid_with_strictness(
  apub_id: &Url,
  is_strict: bool,
  settings: &Settings,
) -> Result<(), LemmyError> {
  check_apub_id_valid(apub_id, settings).map_err(LemmyError::from_message)?;
  let domain = apub_id.domain().expect("apud id has domain").to_string();
  let local_instance = settings
    .get_hostname_without_port()
    .expect("local hostname is valid");
  if domain == local_instance {
    return Ok(());
  }

  if let Some(mut allowed) = settings.to_owned().federation.allowed_instances {
    // Only check allowlist if this is a community, or strict allowlist is enabled.
    let strict_allowlist = settings.to_owned().federation.strict_allowlist;
    if is_strict || strict_allowlist {
      // need to allow this explicitly because apub receive might contain objects from our local
      // instance.
      allowed.push(local_instance);

      if !allowed.contains(&domain) {
        return Err(LemmyError::from_message(
          "Federation forbidden by strict allowlist",
        ));
      }
    }
  }
  Ok(())
}

pub(crate) fn deserialize_one_or_many<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
  T: Deserialize<'de>,
  D: Deserializer<'de>,
{
  #[derive(Deserialize)]
  #[serde(untagged)]
  enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
  }

  let result: OneOrMany<T> = Deserialize::deserialize(deserializer)?;
  Ok(match result {
    OneOrMany::Many(list) => list,
    OneOrMany::One(value) => vec![value],
  })
}

pub(crate) fn deserialize_one<'de, T, D>(deserializer: D) -> Result<[T; 1], D::Error>
where
  T: Deserialize<'de>,
  D: Deserializer<'de>,
{
  #[derive(Deserialize)]
  #[serde(untagged)]
  enum MaybeArray<T> {
    Simple(T),
    Array([T; 1]),
  }

  let result: MaybeArray<T> = Deserialize::deserialize(deserializer)?;
  Ok(match result {
    MaybeArray::Simple(value) => [value],
    MaybeArray::Array(value) => value,
  })
}

pub(crate) fn deserialize_skip_error<'de, T, D>(deserializer: D) -> Result<T, D::Error>
where
  T: Deserialize<'de> + Default,
  D: Deserializer<'de>,
{
  let result = Deserialize::deserialize(deserializer);
  Ok(match result {
    Ok(o) => o,
    Err(_) => Default::default(),
  })
}

pub enum EndpointType {
  Community,
  Person,
  Post,
  Comment,
  PrivateMessage,
}

/// Generates an apub endpoint for a given domain, IE xyz.tld
pub fn generate_local_apub_endpoint(
  endpoint_type: EndpointType,
  name: &str,
  domain: &str,
) -> Result<DbUrl, ParseError> {
  let point = match endpoint_type {
    EndpointType::Community => "c",
    EndpointType::Person => "u",
    EndpointType::Post => "post",
    EndpointType::Comment => "comment",
    EndpointType::PrivateMessage => "private_message",
  };

  Ok(Url::parse(&format!("{}/{}/{}", domain, point, name))?.into())
}

pub fn generate_followers_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  Ok(Url::parse(&format!("{}/followers", actor_id))?.into())
}

pub fn generate_inbox_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  Ok(Url::parse(&format!("{}/inbox", actor_id))?.into())
}

pub fn generate_site_inbox_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  let mut actor_id: Url = actor_id.clone().into();
  actor_id.set_path("site_inbox");
  Ok(actor_id.into())
}

pub fn generate_shared_inbox_url(actor_id: &DbUrl) -> Result<DbUrl, LemmyError> {
  let actor_id: Url = actor_id.clone().into();
  let url = format!(
    "{}://{}{}/inbox",
    &actor_id.scheme(),
    &actor_id.host_str().context(location_info!())?,
    if let Some(port) = actor_id.port() {
      format!(":{}", port)
    } else {
      "".to_string()
    },
  );
  Ok(Url::parse(&url)?.into())
}

pub fn generate_outbox_url(actor_id: &DbUrl) -> Result<DbUrl, ParseError> {
  Ok(Url::parse(&format!("{}/outbox", actor_id))?.into())
}

fn generate_moderators_url(community_id: &DbUrl) -> Result<DbUrl, LemmyError> {
  Ok(Url::parse(&format!("{}/moderators", community_id))?.into())
}

/// Store a sent or received activity in the database, for logging purposes. These records are not
/// persistent.
#[tracing::instrument(skip(pool))]
async fn insert_activity(
  ap_id: &Url,
  activity: serde_json::Value,
  local: bool,
  sensitive: bool,
  pool: &DbPool,
) -> Result<bool, LemmyError> {
  let ap_id = ap_id.to_owned().into();
  Ok(
    blocking(pool, move |conn| {
      Activity::insert(conn, ap_id, activity, local, sensitive)
    })
    .await??,
  )
}

/// Common methods provided by ActivityPub actors (community and person). Not all methods are
/// implemented by all actors.
pub trait ActorType: ActorPublicKey {
  fn actor_id(&self) -> Url;

  fn private_key(&self) -> Option<String>;

  fn inbox_url(&self) -> Url;

  fn shared_inbox_url(&self) -> Option<Url>;

  fn shared_inbox_or_inbox_url(&self) -> Url {
    self.shared_inbox_url().unwrap_or_else(|| self.inbox_url())
  }

  fn get_public_key(&self) -> PublicKey {
    PublicKey::new_main_key(self.actor_id(), self.public_key().to_string())
  }
}
