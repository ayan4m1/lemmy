use crate::{
  activity_lists::SiteInboxActivities,
  context::WithContext,
  http::{create_apub_response, receive_lemmy_activity},
  objects::instance::ApubSite,
  protocol::collections::empty_outbox::EmptyOutbox,
};
use actix_web::{web, HttpRequest, HttpResponse};
use lemmy_api_common::utils::blocking;
use lemmy_apub_lib::traits::ApubObject;
use lemmy_db_schema::source::site::Site;
use lemmy_utils::{settings::structs::Settings, LemmyError};
use lemmy_websocket::LemmyContext;
use url::Url;

pub(crate) async fn get_apub_site_http(
  context: web::Data<LemmyContext>,
) -> Result<HttpResponse, LemmyError> {
  let site: ApubSite = blocking(context.pool(), Site::read_local_site)
    .await??
    .into();

  let apub = site.into_apub(&context).await?;
  Ok(create_apub_response(&apub))
}

#[tracing::instrument(skip_all)]
pub(crate) async fn get_apub_site_outbox() -> Result<HttpResponse, LemmyError> {
  let outbox_id = format!(
    "{}/site_outbox",
    Settings::get().get_protocol_and_hostname()
  );
  let outbox = EmptyOutbox::new(Url::parse(&outbox_id)?).await?;
  Ok(create_apub_response(&outbox))
}

#[tracing::instrument(skip_all)]
pub async fn get_apub_site_inbox(
  request: HttpRequest,
  payload: String,
  context: web::Data<LemmyContext>,
) -> Result<HttpResponse, LemmyError> {
  receive_lemmy_activity::<WithContext<SiteInboxActivities>, ApubSite>(request, payload, context)
    .await
}
