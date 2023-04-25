use crate::Perform;
use actix_web::web::Data;
use lemmy_api_common::{
  context::LemmyContext,
  post::{ListPostReports, ListPostReportsResponse},
  sensitive::Sensitive,
  utils::local_user_view_from_jwt_new,
};
use lemmy_db_views::post_report_view::PostReportQuery;
use lemmy_utils::{error::LemmyError, ConnectionId};

/// Lists post reports for a community if an id is supplied
/// or returns all post reports for communities a user moderates
#[async_trait::async_trait(?Send)]
impl Perform for ListPostReports {
  type Response = ListPostReportsResponse;

  #[tracing::instrument(skip(context, _websocket_id))]
  async fn perform(
    &self,
    context: &Data<LemmyContext>,
    auth: Option<Sensitive<String>>,
    _websocket_id: Option<ConnectionId>,
  ) -> Result<ListPostReportsResponse, LemmyError> {
    let data: &ListPostReports = self;
    let local_user_view = local_user_view_from_jwt_new(auth, context).await?;

    let person_id = local_user_view.person.id;
    let admin = local_user_view.person.admin;
    let community_id = data.community_id;
    let unresolved_only = data.unresolved_only;

    let page = data.page;
    let limit = data.limit;
    let post_reports = PostReportQuery::builder()
      .pool(context.pool())
      .my_person_id(person_id)
      .admin(admin)
      .community_id(community_id)
      .unresolved_only(unresolved_only)
      .page(page)
      .limit(limit)
      .build()
      .list()
      .await?;

    let res = ListPostReportsResponse { post_reports };

    Ok(res)
  }
}
