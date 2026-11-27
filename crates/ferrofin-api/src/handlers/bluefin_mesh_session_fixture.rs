//! Stateful session cleanup fixture for mesh authority tests only.
//! Other operations retain the routing fixture's behavior and fail if used.
use std::sync::Mutex;
use async_trait::async_trait;
use ferrofin_db::entities::{security::DeviceEntity, users::UserEntity};
use ferrofin_model::{dto::SessionInfoDto, session::{
    ClientCapabilities, GeneralCommand, MessageCommand, PlayRequest,
    PlaybackProgressInfo, PlaybackStartInfo, PlaybackStopInfo, PlaystateRequest,
    SessionMessageType, TranscodingInfo,
}};
use ferrofin_traits::{error::ServiceError, session::{
    AuthenticationRequest, AuthenticationResultData, SessionManager,
}};
use uuid::Uuid;
use crate::test_support::FAKE_ACCESS_TOKEN;

#[derive(Default)]
pub(super) struct Sessions { state: Mutex<State> }
#[derive(Default)]
struct State {
    sessions: Vec<SessionInfoDto>,
    close_attempts: Vec<String>,
    fail_next_close: bool,
    mint_attempts: usize,
}
impl Sessions {
    pub(super) fn add(&self, id: &str, user_id: Uuid, device_id: &str) {
        self.state.lock().unwrap().sessions.push(SessionInfoDto {
            id: Some(id.into()), user_id, device_id: Some(device_id.into()),
            ..Default::default()
        });
    }
    pub(super) fn contains(&self, id: &str) -> bool {
        self.state.lock().unwrap().sessions.iter()
            .any(|session| session.id.as_deref() == Some(id))
    }
    pub(super) fn fail_next_close(&self) {
        self.state.lock().unwrap().fail_next_close = true;
    }
    pub(super) fn close_attempts(&self) -> Vec<String> {
        self.state.lock().unwrap().close_attempts.clone()
    }
    pub(super) fn mint_attempts(&self) -> usize {
        self.state.lock().unwrap().mint_attempts
    }
}
#[async_trait]
impl SessionManager for Sessions {
    async fn authenticate_mesh(
        &self, _request: &AuthenticationRequest,
    ) -> Result<AuthenticationResultData, ServiceError> {
        self.state.lock().unwrap().mint_attempts += 1;
        Err(ServiceError::Unavailable("No test bearer was provisioned".into()))
    }
    async fn log_session_activity(
        &self,
        _app_name: &str,
        _app_version: &str,
        _device_id: &str,
        _device_name: &str,
        _remote_endpoint: &str,
        _user: Option<&UserEntity>,
    ) -> Result<SessionInfoDto, ServiceError> {
        unimplemented!("fake")
    }
    async fn update_device_name(
        &self,
        _session_id: &str,
        _reported_device_name: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn on_playback_start(&self, _info: &PlaybackStartInfo) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn on_playback_progress(
        &self,
        _info: &PlaybackProgressInfo,
        _is_automated: bool,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn on_playback_stopped(&self, _info: &PlaybackStopInfo) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn report_session_ended(&self, _session_id: &str) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn close_if_needed(&self, session_id: &str) -> Result<(), ServiceError> {
        let mut state = self.state.lock().unwrap();
        state.close_attempts.push(session_id.to_owned());
        if std::mem::take(&mut state.fail_next_close) {
            return Err(ServiceError::backend("injected playback cleanup failure"));
        }
        state.sessions.retain(|session| session.id.as_deref() != Some(session_id));
        Ok(())
    }
    async fn send_general_command(
        &self,
        _controlling_session_id: &str,
        _session_id: &str,
        _command: &GeneralCommand,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_message_command(
        &self,
        _controlling_session_id: &str,
        _session_id: &str,
        _command: &MessageCommand,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_play_command(
        &self,
        _controlling_session_id: &str,
        _session_id: &str,
        _command: &PlayRequest,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_playstate_command(
        &self,
        _controlling_session_id: &str,
        _session_id: &str,
        _command: &PlaystateRequest,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_message_to_admin_sessions(
        &self,
        _message_type: SessionMessageType,
        _data: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_message_to_user_sessions(
        &self,
        _user_ids: &[Uuid],
        _message_type: SessionMessageType,
        _data: &str,
    ) -> Result<(), ServiceError> {
        // The user-data manager's batched `UserDataChanged` goes out here;
        // delivery is covered by ferrofin-core's tests.
        Ok(())
    }
    async fn send_message_to_user_device_sessions(
        &self,
        _device_id: &str,
        _message_type: SessionMessageType,
        _data: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn send_restart_required_notification(&self) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn add_additional_user(
        &self,
        _session_id: &str,
        _user_id: Uuid,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn remove_additional_user(
        &self,
        _session_id: &str,
        _user_id: Uuid,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn report_now_viewing_item(
        &self,
        _session_id: &str,
        _item_id: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn authenticate_new_session(
        &self,
        _request: &AuthenticationRequest,
    ) -> Result<AuthenticationResultData, ServiceError> {
        Ok(AuthenticationResultData {
            session: SessionInfoDto::default(),
            access_token: FAKE_ACCESS_TOKEN.into(),
        })
    }
    async fn authenticate_direct(
        &self,
        _request: &AuthenticationRequest,
    ) -> Result<AuthenticationResultData, ServiceError> {
        Ok(AuthenticationResultData {
            session: SessionInfoDto::default(),
            access_token: FAKE_ACCESS_TOKEN.into(),
        })
    }
    async fn report_capabilities(
        &self,
        _session_id: &str,
        _capabilities: &ClientCapabilities,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn report_transcoding_info(
        &self,
        _device_id: &str,
        _info: &TranscodingInfo,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn clear_transcoding_info(&self, _device_id: &str) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn get_sessions(
        &self,
        _user_id: Uuid,
        device_id: Option<&str>,
        active_within_seconds: Option<i32>,
        controllable_user_to_check: Option<Uuid>,
        is_api_key: bool,
    ) -> Result<Vec<SessionInfoDto>, ServiceError> {
        // Match the production privileged lookup: device filtering can return
        // another user's session. The revoker must narrow the account itself.
        assert!(is_api_key && active_within_seconds.is_none()
            && controllable_user_to_check.is_none());
        let state = self.state.lock().unwrap();
        Ok(state.sessions.iter().filter(|session| device_id.is_none_or(|wanted|
            session.device_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(wanted))
        )).cloned().collect())
    }
    async fn get_session_by_authentication_token(
        &self,
        _token: &str,
        _device_id: &str,
        _remote_endpoint: &str,
    ) -> Result<SessionInfoDto, ServiceError> {
        unimplemented!("fake")
    }
    async fn logout(&self, _access_token: &str) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn logout_device(&self, _device: &DeviceEntity) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn revoke_user_tokens(
        &self,
        _user_id: Uuid,
        _current_access_token: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
    async fn close_live_stream_if_needed(
        &self,
        _live_stream_id: &str,
        _session_or_play_session_id: &str,
    ) -> Result<(), ServiceError> {
        unimplemented!("fake")
    }
}
