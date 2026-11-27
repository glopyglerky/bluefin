//! Exercises real passkey and ML-DSA proofs against the production SQL owner.
use super::*;
#[path = "bluefin_mesh_session_fixture.rs"]
mod session_fixture;

fn relay_store()->Mutex<bluefin_mesh_connect::relay_admission::RelayAdmissions>{
    Mutex::new(bluefin_mesh_connect::relay_admission::RelayAdmissions::new("https://relay.example/").unwrap())
}
async fn relay_fixture()->Fixture {
    let mut f=Fixture::new().await;
    let mut published=address();published.relay_urls=vec!["https://relay.example/".into()];
    f.service=Service::new(f.db.clone(),"https://accounts.example",published).await.unwrap();
    f
}

#[tokio::test]
async fn mesh_relay_bootstrap_requires_exact_live_capsule_and_never_creates_a_session(){
    use std::time::{Duration,Instant};
    let f=relay_fixture().await;let store=relay_store();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let mut forged=invite.clone();forged.secret[0]^=1;
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&forged,[9;32]).await.is_err());
    assert!(!store.lock().await.allows(&[9;32],Instant::now()));
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,invite.endpoint_id).await.is_err());
    f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.unwrap();
    assert!(store.lock().await.allows(&[9;32],Instant::now()));
    assert!(!store.lock().await.allows(&[9;32],Instant::now()+Duration::from_secs(120)));
    assert_eq!(f.service.invite_row(&invite).await.unwrap().get::<String,_>("State"),"issued");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),0);
    let _new=Service::new(f.db.clone(),"https://accounts.example",f.service.address.clone()).await.unwrap();
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[8;32]).await.is_err());
    assert!(!store.lock().await.allows(&[8;32],Instant::now()));
}

#[tokio::test]
async fn mesh_relay_bootstrap_rechecks_policy_and_operation_budget(){
    let f=relay_fixture().await;let store=relay_store();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let permit=f.service.operations.acquire_many(16).await.unwrap();
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.is_err());
    drop(permit);
    let temporary=f.directory.path().join("accounts-policy.tmp");
    std::fs::write(&temporary,r#"{"allowSignups":false}"#).unwrap();
    std::fs::rename(&temporary,f.directory.path().join("bluefin-accounts.json")).unwrap();
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.is_err());
    assert!(!store.lock().await.allows(&[9;32],std::time::Instant::now()));
}

#[tokio::test]
async fn mesh_relay_bootstrap_checks_login_account_policy(){
    let f=relay_fixture().await;let store=relay_store();
    let (user,_)=f.login_account().await;
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.unwrap();
    // Use the same account policy owner as ordinary remote authorization.
    let account=f.state.users.get_user_by_id(user).await.unwrap().unwrap();
    let mut policy=f.state.users.get_user_dto(&account,None).await.unwrap().policy.unwrap();
    policy.enable_remote_access=false;
    f.state.users.update_policy(user,&policy).await.unwrap();
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[8;32]).await.is_err());
}

#[tokio::test]
async fn mesh_relay_bootstrap_claim_and_expiry_cannot_be_bypassed(){
    let f=relay_fixture().await;let store=relay_store();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let device=DeviceIdentity::from_seed([67;32]).unwrap();
    let start=begin(&f,invite.clone(),&device,1,None).await;
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.is_err());
    assert!(f.service.handle(&f.state,&device.public_key(),finish_request(&start,&device,2,json!({}))).await.is_err());
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.is_err());
    let fresh=f.invite(InviteIntent::Signup,None).await;
    sqlx::query("UPDATE BluefinMeshInvites SET Expires=? WHERE Id=?")
        .bind(now()-1).bind(fresh.invitation_id.to_vec()).execute(f.db.writer()).await.unwrap();
    assert!(f.service.admit_relay_bootstrap(&f.state,&store,&fresh,[9;32]).await.is_err());
    assert!(!store.lock().await.allows(&[9;32],std::time::Instant::now()));
}

#[tokio::test]
async fn mesh_relay_bootstrap_rechecks_ledger_after_waiting_for_store(){
    use std::{future::{Future,poll_fn},task::Poll};
    for expire in [false,true] {
        let f=relay_fixture().await;let store=relay_store();
        let invite=f.invite(InviteIntent::Signup,None).await;
        let lock=store.lock().await;
        let mut attempt=Box::pin(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]));
        // Drive past the initial ledger read to the store lock, without timers
        // or depending on scheduler ordering. Once the SQL reader completes,
        // the pending future holds both service and account-policy gates.
        poll_fn(|cx|{
            assert!(attempt.as_mut().poll(cx).is_pending());
            if f.service.gate.try_lock().is_err() && f.state.account_gate.try_lock().is_err() {
                // The final check is after the store lock regardless of how
                // far the asynchronous initial SQL read has progressed.
                Poll::Ready(())
            } else {cx.waker().wake_by_ref();Poll::Pending}
        }).await;
        if expire {
            sqlx::query("UPDATE BluefinMeshInvites SET Expires=? WHERE Id=?")
                .bind(now()-1).bind(invite.invitation_id.to_vec()).execute(f.db.writer()).await.unwrap();
        } else {
            sqlx::query("UPDATE BluefinMeshInvites SET State='revoked' WHERE Id=?")
                .bind(invite.invitation_id.to_vec()).execute(f.db.writer()).await.unwrap();
        }
        drop(lock);
        assert!(attempt.await.is_err());
        assert!(!store.lock().await.allows(&[9;32],std::time::Instant::now()));
    }
}

#[tokio::test]
async fn mesh_relay_bootstrap_obeys_remote_network_policy(){
    use ferrofin_networking::{NetworkConfiguration,NetworkManager};
    let mut f=relay_fixture().await;let store=relay_store();
    let invite=f.invite(InviteIntent::Signup,None).await;
    for configuration in [
        NetworkConfiguration{enable_remote_access:false,..Default::default()},
        NetworkConfiguration{enable_remote_access:true,remote_ip_filter:vec!["192.0.2.0/24".into()],..Default::default()},
    ] {
        let mut inner=(*f.state).clone();
        inner.network=Some(Arc::new(std::sync::RwLock::new(NetworkManager::with_defaults(configuration,""))));
        f.state=AppState::from_inner(inner);
        assert!(f.service.admit_relay_bootstrap(&f.state,&store,&invite,[9;32]).await.is_err());
    }
}

#[tokio::test]
async fn mesh_relay_bootstrap_drives_the_private_http_callout_only(){
    use axum::{body::Body,http::{Request as HttpRequest,StatusCode}};
    use tower::ServiceExt;
    let f=relay_fixture().await;
    let relay=super::super::bluefin_mesh_relay::RelayCallout::new("https://relay.example/",&"ab".repeat(32)).unwrap();
    let router=relay.clone().router();
    let request=||HttpRequest::builder().method("POST").uri("/relay/admission")
        .header("authorization",format!("Bearer {}","ab".repeat(32)))
        .header("x-iroh-endpoint-id","09".repeat(32)).body(Body::empty()).unwrap();
    assert_eq!(router.clone().oneshot(request()).await.unwrap().status(),StatusCode::FORBIDDEN);
    let invite=f.invite(InviteIntent::Signup,None).await;
    f.service.admit_relay_bootstrap(&f.state,&relay.admissions,&invite,[9;32]).await.unwrap();
    assert_eq!(router.clone().oneshot(request()).await.unwrap().status(),StatusCode::OK);
    relay.admissions.lock().await.revoke_invitation(&invite.invitation_id);
    assert_eq!(router.oneshot(request()).await.unwrap().status(),StatusCode::FORBIDDEN);
}

use async_trait::async_trait;
use bluefin_mesh_connect::native::DeviceIdentity;
use ferrofin_core::user_manager::FerrofinUserManager;
use ferrofin_model::{branding::BrandingOptions, configuration::ServerConfiguration};
use ferrofin_traits::{configuration::ServerConfigurationManager, library::UserManager, system::ServerApplicationPaths};
use webauthn_authenticator_rs::{WebauthnAuthenticator, softpasskey::SoftPasskey};

struct Paths(String);
macro_rules! paths {($($name:ident),*)=>{$(fn $name(&self)->String{self.0.clone()})*};}
impl ServerApplicationPaths for Paths {
    paths!(root_folder_path,default_user_views_path,people_path,genre_path,music_genre_path,studio_path,
        year_path,artists_path,user_configuration_directory_path,configuration_directory_path,
        internal_metadata_path,program_data_path,web_path,data_path,image_cache_path,cache_path,log_directory_path);
}
struct Config(Arc<Paths>);
#[async_trait]
impl ServerConfigurationManager for Config {
    fn application_paths(&self)->Arc<dyn ServerApplicationPaths>{self.0.clone()}
    async fn configuration(&self)->Result<Arc<ServerConfiguration>,ServiceError>{
        Ok(Arc::new(ServerConfiguration{is_startup_wizard_completed:true,..Default::default()}))
    }
    async fn update_configuration(&self,_:&ServerConfiguration)->Result<(),ServiceError>{Ok(())}
    async fn get_branding(&self)->Result<BrandingOptions,ServiceError>{Ok(Default::default())}
    async fn update_branding(&self,_:&BrandingOptions)->Result<(),ServiceError>{Ok(())}
}
struct Fixture { directory:tempfile::TempDir, state:AppState, service:Arc<Service>, db:Database,
    sessions:Arc<session_fixture::Sessions> }
impl Fixture {
    async fn new()->Self {
        let directory=tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"manual"}"#).unwrap();
        let db=Database::connect_in_memory().await.unwrap();db.run_migrations().await.unwrap();
        let mut inner=(*crate::test_support::fake_state()).clone();
        let sessions=Arc::new(session_fixture::Sessions::default());
        inner.sessions=sessions.clone();
        inner.users=Arc::new(FerrofinUserManager::new(db.clone()));
        inner.config=Arc::new(Config(Arc::new(Paths(directory.path().to_string_lossy().into_owned()))));
        let state=AppState::from_inner(inner);
        let service=Service::new(db.clone(),"https://accounts.example",address()).await.unwrap();
        Self{directory,state,service,db,sessions}
    }
    async fn login_account(&self)->(Uuid,WebauthnAuthenticator<SoftPasskey>){
        let user=self.state.users.create_signup_user("viewer","A long test password only").await.unwrap();
        let id=Uuid::parse_str(&user.id).unwrap();
        let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
        let (options,private)=self.service.webauthn.start_passkey_registration(id,"viewer","viewer",None).unwrap();
        let proof=authenticator.do_registration(Url::parse("https://accounts.example").unwrap(),options).unwrap();
        let key=self.service.webauthn.finish_passkey_registration(&proof,&private).unwrap();
        sqlx::query("INSERT INTO BluefinPasskeys(CredentialId,UserId,Credential) VALUES(?,?,?)")
            .bind(key.cred_id().as_ref()).bind(guid_to_db(id)).bind(serde_json::to_string(&key).unwrap())
            .execute(self.db.writer()).await.unwrap();
        (id,authenticator)
    }
    async fn invite(&self,intent:InviteIntent,user:Option<Uuid>)->ConnectInvite{
        let result=self.service.issue(&self.state,intent,user,600).await.unwrap();
        ConnectInvite::parse(result["code"].as_str().unwrap()).unwrap()
    }
}
fn address()->ServerAddress {
    ServerAddress {endpoint_id:[1;32],server_certificate_sha256:[2;32],service:SERVICE.into(),
        direct_addresses:vec!["127.0.0.1:49215".parse().unwrap()],relay_urls:vec![]}
}
fn envelope(request:Request)->Value{serde_json::to_value(Envelope::new(request)).unwrap()}
async fn begin(f:&Fixture,invite:ConnectInvite,device:&DeviceIdentity,id:u8,name:Option<&str>)->Value{
    f.service.handle(&f.state,&device.public_key(),envelope(Request::Begin{
        request_id:[id;32],invitation:invite,device_public_key:device.public_key(),
        device_label:"Test device".into(),account_name:name.map(str::to_owned),
    })).await.unwrap()
}
fn finish_request(begin:&Value,device:&DeviceIdentity,id:u8,passkey:Value)->Value{
    envelope(Request::Finish{request_id:[id;32],challenge_id:begin["body"]["challenge_id"].as_str().unwrap().into(),
        passkey_response:passkey,device_signature:device.sign(begin["body"]["device_proof_message"].as_str().unwrap().as_bytes()).unwrap()})
}
fn assertion(authenticator:&mut WebauthnAuthenticator<SoftPasskey>,begin:&Value)->Value {
    let options=serde_json::from_value(json!({"publicKey":begin["body"]["passkey_options"]["publicKey"]})).unwrap();
    serde_json::to_value(authenticator.do_authentication(Url::parse("https://accounts.example").unwrap(),options).unwrap()).unwrap()
}
fn registration(authenticator:&mut WebauthnAuthenticator<SoftPasskey>,begin:&Value)->Value {
    let options=serde_json::from_value(json!({"publicKey":begin["body"]["passkey_options"]["publicKey"]})).unwrap();
    serde_json::to_value(authenticator.do_registration(Url::parse("https://accounts.example").unwrap(),options).unwrap()).unwrap()
}
#[tokio::test]
async fn mesh_login_claims_once_across_devices_and_only_exact_device_retry_recovers(){
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    let first=DeviceIdentity::from_seed([7;32]).unwrap();
    let second=DeviceIdentity::from_seed([8;32]).unwrap();
    let start=|device:&DeviceIdentity,id:u8|envelope(Request::Begin{request_id:[id;32],
        invitation:invite.clone(),device_public_key:device.public_key(),device_label:"Test device".into(),account_name:None});
    let ar=start(&first,1);let br=start(&second,2);
    let first_key=first.public_key();let second_key=second.public_key();
    let (a,b)=tokio::join!(f.service.handle(&f.state,&first_key,ar.clone()),f.service.handle(&f.state,&second_key,br.clone()));
    assert_ne!(a.is_ok(),b.is_ok(),"only one device may claim a code");
    let (device,key,other,begin_request,begin)=if let Ok(a)=a {(first,first_key,second_key,ar,a)}else{(second,second_key,first_key,br,b.unwrap())};
    assert_eq!(f.service.handle(&f.state,&key,begin_request.clone()).await.unwrap(),begin);
    assert!(f.service.handle(&f.state,&other,begin_request).await.is_err());
    assert!(sqlx::query("UPDATE BluefinMeshInvites SET State='issued'").execute(f.db.writer()).await.is_err());
    let request=finish_request(&begin,&device,3,assertion(&mut authenticator,&begin));
    let (a,b)=tokio::join!(f.service.handle(&f.state,&key,request.clone()),f.service.handle(&f.state,&key,request.clone()));
    let result=a.unwrap();assert_eq!(result,b.unwrap());
    assert!(f.service.handle(&f.state,&other,request.clone()).await.is_err());
    let mut changed=request;let byte=changed["body"]["device_signature"][0].as_u64().unwrap();
    changed["body"]["device_signature"][0]=json!(byte^1);
    assert!(f.service.handle(&f.state,&key,changed).await.is_err());
    assert!(!serde_json::to_string(&result).unwrap().contains("AccessToken"));
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),1);
    assert!(f.service.invite_row(&invite).await.is_err());
    assert!(sqlx::query("UPDATE BluefinMeshInvites SET State='issued'").execute(f.db.writer()).await.is_err());
}
#[tokio::test]
async fn mesh_finish_transaction_abort_preserves_the_consumed_browser_proof_for_exact_retry(){
    use tower::ServiceExt;
    use axum::{body::Body,http::Request as HttpRequest};
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let device=DeviceIdentity::from_seed([71;32]).unwrap();
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    let start=begin(&f,invite.clone(),&device,1,None).await;
    let request=finish_request(&start,&device,2,json!({"browser":true}));
    let url=Url::parse(start["body"]["passkey_options"]["browserUrl"].as_str().unwrap()).unwrap();
    let (challenge,secret)=url.fragment().unwrap().split_once('.').unwrap();
    let body=json!({"challenge":challenge,"secret":secret,"credential":assertion(&mut authenticator,&start)});
    let router=web::routes().with_state(f.state.clone().with_mesh_accounts(f.service.clone()));
    let post=||HttpRequest::builder().method("POST").uri("/Bluefin/Accounts/Mesh/Passkey/Verify")
        .header("content-type","application/json").header("origin","https://accounts.example")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
    assert_eq!(router.clone().oneshot(post()).await.unwrap().status(),axum::http::StatusCode::NO_CONTENT);
    let credential:String=sqlx::query_scalar("SELECT Credential FROM BluefinPasskeys")
        .fetch_one(f.db.pool()).await.unwrap();
    // Abort after the grant and credential update, before the receipt can commit.
    sqlx::query("CREATE TRIGGER FailMeshFinishReceipt BEFORE INSERT ON BluefinMeshReceipts BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END")
        .execute(f.db.writer()).await.unwrap();
    assert!(f.service.handle(&f.state,&device.public_key(),request.clone()).await.is_err());
    sqlx::query("DROP TRIGGER FailMeshFinishReceipt").execute(f.db.writer()).await.unwrap();
    assert_eq!(f.service.invite_row(&invite).await.unwrap().get::<String,_>("State"),"claimed");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),0);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(f.db.pool()).await.unwrap(),0);
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT Credential FROM BluefinPasskeys").fetch_one(f.db.pool()).await.unwrap(),credential);
    // The one-use browser proof stays consumed, but the signed original request
    // still owns it. Neither a changed request nor another device may take over.
    assert!(!router.oneshot(post()).await.unwrap().status().is_success());
    let changed=finish_request(&start,&device,3,json!({"browser":true}));
    assert!(f.service.handle(&f.state,&device.public_key(),changed).await.is_err());
    let mut changed=request.clone();changed["body"]["passkey_response"]=json!({});
    assert!(f.service.handle(&f.state,&device.public_key(),changed).await.is_err());
    let other=DeviceIdentity::from_seed([72;32]).unwrap();
    assert!(f.service.handle(&f.state,&other.public_key(),request.clone()).await.is_err());
    let key=device.public_key();
    let (first,retry)=tokio::join!(
        f.service.handle(&f.state,&key,request.clone()),
        f.service.handle(&f.state,&key,request));
    let completed=first.unwrap();assert_eq!(retry.unwrap(),completed);
    assert_eq!(completed["body"]["session"]["kind"],"session");
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(f.db.pool()).await.unwrap(),1);
    assert!(!f.service.pending.lock().await.contains_key(challenge));
}

#[tokio::test]
async fn mesh_cancelled_finish_keeps_the_exact_attempt_for_retry(){
    use std::{future::{Future,poll_fn},task::Poll};
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let device=DeviceIdentity::from_seed([73;32]).unwrap();
    let start=begin(&f,f.invite(InviteIntent::Login,Some(user)).await,&device,1,None).await;
    let request=finish_request(&start,&device,2,assertion(&mut authenticator,&start));
    let envelope:Envelope<Request>=serde_json::from_value(request.clone()).unwrap();
    let request_hash=digest(&serde_json::to_vec(&envelope.body).unwrap());
    let Request::Finish{request_id,challenge_id,passkey_response,device_signature}=envelope.body else {panic!("finish request")};
    let key=device.public_key();
    // The in-memory fixture uses one shared reader/writer connection. Holding
    // it suspends Finish at its first DB read, after accepting the device proof.
    let connection=f.db.writer().acquire().await.unwrap();
    let mut attempt=Box::pin(f.service.finish(&f.state,&key,request_id,request_hash,&challenge_id,passkey_response,&device_signature));
    poll_fn(|cx|{
        assert!(attempt.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    }).await;
    drop(attempt);
    drop(connection);
    {
        let pending=f.service.pending.lock().await;
        assert_eq!(pending.get(&challenge_id).unwrap().finish_attempt,Some((request_id,request_hash)));
    }
    let changed=finish_request(&start,&device,3,json!({}));
    assert!(f.service.handle(&f.state,&key,changed).await.is_err());
    let completed=f.service.handle(&f.state,&key,request.clone()).await.unwrap();
    assert_eq!(completed["body"]["session"]["kind"],"session");
    assert_eq!(f.service.handle(&f.state,&key,request).await.unwrap(),completed);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),1);
}

#[tokio::test]
async fn mesh_invalid_device_proof_cannot_spend_or_steal_an_invitation(){
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let other=DeviceIdentity::from_seed([8;32]).unwrap();
    let start=begin(&f,invite.clone(),&device,1,None).await;
    let request=finish_request(&start,&other,3,assertion(&mut authenticator,&start));
    assert!(f.service.handle(&f.state,&device.public_key(),request).await.is_err());
    assert!(f.service.invite_row(&invite).await.is_ok());
}
#[tokio::test]
async fn mesh_signup_and_later_login_do_not_require_an_account_name(){
    let f=Fixture::new().await;
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let signup=f.invite(InviteIntent::Signup,None).await;
    let start=begin(&f,signup,&device,1,None).await;
    let key_user=&start["body"]["passkey_options"]["publicKey"]["user"];
    let generated=key_user["name"].as_str().unwrap();
    assert!(Uuid::parse_str(generated.strip_prefix("bluefin-").unwrap()).is_ok());
    assert_eq!(key_user["displayName"],"Bluefin account");
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let proof=registration(&mut authenticator,&start);
    let finish=f.service.handle(&f.state,&device.public_key(),finish_request(&start,&device,2,proof)).await.unwrap();
    let handle=finish["body"]["session"]["handle"].as_str().unwrap();
    let user=f.state.users.approve_mesh_signup(handle).await.unwrap();
    assert_eq!(user.username,generated);
    let login=f.invite(InviteIntent::Login,Some(Uuid::parse_str(&user.id).unwrap())).await;
    let other=DeviceIdentity::from_seed([8;32]).unwrap();
    let start=begin(&f,login,&other,3,None).await;
    let proof=assertion(&mut authenticator,&start);
    let finish=f.service.handle(&f.state,&other.public_key(),finish_request(&start,&other,4,proof)).await.unwrap();
    assert_eq!(finish["body"]["session"]["kind"],"session");
}
#[tokio::test]
async fn mesh_signup_waits_for_owner_approval_and_the_original_device_resumes(){
    let f=Fixture::new().await;
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let start=begin(&f,invite.clone(),&device,1,Some("newviewer")).await;
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let proof=registration(&mut authenticator,&start);
    let request=finish_request(&start,&device,2,proof);
    let finish=f.service.handle(&f.state,&device.public_key(),request.clone()).await.unwrap();
    let handle=finish["body"]["session"]["handle"].as_str().unwrap();
    assert_eq!(finish["body"]["session"]["state"],"pending");
    assert!(f.state.users.get_user_by_name("newviewer").await.unwrap().is_none());
    assert!(f.service.invite_row(&invite).await.is_err());
    assert_eq!(f.service.handle(&f.state,&device.public_key(),request).await.unwrap(),finish);
    assert!(f.service.status(&f.state,&DeviceIdentity::from_seed([8;32]).unwrap().public_key(),handle).await.is_err());
    let user=f.state.users.approve_mesh_signup(handle).await.unwrap();
    let policy=f.state.users.get_user_dto(&user,None).await.unwrap().policy.unwrap();
    assert!(!policy.is_administrator&&!policy.enable_all_folders&&!policy.enable_public_sharing);
    assert!(user.password.as_ref().is_some_and(|p|!p.is_empty()));
    let state=f.service.status(&f.state,&device.public_key(),handle).await.unwrap();
    assert_eq!(state["body"]["state"],"approved");
    assert!(state["body"]["session"]["id"].is_string());
    assert_eq!(f.service.status(&f.state,&device.public_key(),handle).await.unwrap(),state);
    assert_eq!(f.state.users.approve_mesh_signup(handle).await.unwrap().id,user.id);
}
#[tokio::test]
async fn mesh_autoapproval_is_server_policy_and_never_grants_administration(){
    let f=Fixture::new().await;
    std::fs::write(f.directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let start=begin(&f,invite,&device,1,Some("autoviewer")).await;
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let request=finish_request(&start,&device,2,registration(&mut authenticator,&start));
    let finish=f.service.handle(&f.state,&device.public_key(),request).await.unwrap();
    let handle=finish["body"]["session"]["handle"].as_str().unwrap();
    let status=f.service.status(&f.state,&device.public_key(),handle).await.unwrap();
    assert_eq!(status["body"]["state"],"approved");
    let user=f.state.users.get_user_by_name("autoviewer").await.unwrap().unwrap();
    assert!(!f.state.users.get_user_dto(&user,None).await.unwrap().policy.unwrap().is_administrator);
}
async fn committed_finish_receipt(f:&Fixture)->Value{
    tokio::time::timeout(std::time::Duration::from_secs(10),async{
        loop{
            if let Some(saved)=sqlx::query_scalar::<_,String>("SELECT Response FROM BluefinMeshReceipts LIMIT 1")
                .fetch_optional(f.db.pool()).await.unwrap(){
                return serde_json::from_str(&saved).unwrap();
            }
            tokio::task::yield_now().await;
        }
    }).await.expect("Finish did not commit its receipt before optional approval")
}

#[tokio::test]
async fn mesh_autoapproval_resumes_exact_receipt_after_postcommit_cancellation(){
    let f=Fixture::new().await;let device=DeviceIdentity::from_seed([74;32]).unwrap();
    std::fs::write(f.directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
    let start=begin(&f,f.invite(InviteIntent::Signup,None).await,&device,1,Some("resumedviewer")).await;
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let request=finish_request(&start,&device,2,registration(&mut authenticator,&start));
    let key=device.public_key();
    // This is the real settings-write gate. Finish may commit, but optional
    // provisioning cannot read policy or create an account until it is released.
    let policy_gate=f.state.account_gate.lock().await;
    let mut attempt=Box::pin(f.service.handle(&f.state,&key,request.clone()));
    let receipt=tokio::select!{
        _=&mut attempt=>panic!("Finish returned before the held policy gate"),
        receipt=committed_finish_receipt(&f)=>receipt,
    };
    drop(attempt);drop(policy_gate);
    let handle=receipt["body"]["session"]["handle"].as_str().unwrap();
    assert_eq!(sqlx::query_scalar::<_,String>("SELECT State FROM BluefinMeshSignups WHERE Id=?")
        .bind(handle).fetch_one(f.db.pool()).await.unwrap(),"pending");
    assert!(f.state.users.get_user_by_name("resumedviewer").await.unwrap().is_none());
    let (first,retry)=tokio::join!(
        f.service.handle(&f.state,&key,request.clone()),
        f.service.handle(&f.state,&key,request.clone()));
    assert_eq!(first.unwrap(),receipt);assert_eq!(retry.unwrap(),receipt);
    let user=f.state.users.get_user_by_name("resumedviewer").await.unwrap().unwrap();
    let policy=f.state.users.get_user_dto(&user,None).await.unwrap().policy.unwrap();
    assert!(!policy.is_administrator&&!policy.enable_all_folders&&!policy.enable_public_sharing);
    let status=f.service.status(&f.state,&key,handle).await.unwrap();
    assert_eq!(status["body"]["state"],"approved");
    assert_eq!(f.service.status(&f.state,&key,handle).await.unwrap(),status);
    assert_eq!(f.service.handle(&f.state,&key,request).await.unwrap(),receipt);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(f.db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSignups").fetch_one(f.db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinPasskeys").fetch_one(f.db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),1);
}

#[tokio::test]
async fn mesh_autoapproval_policy_read_failure_preserves_receipt_and_status_recovers(){
    let f=Fixture::new().await;let device=DeviceIdentity::from_seed([75;32]).unwrap();
    let settings=f.directory.path().join("bluefin-accounts.json");
    std::fs::write(&settings,r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
    let start=begin(&f,f.invite(InviteIntent::Signup,None).await,&device,1,Some("policyrecovery")).await;
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let request=finish_request(&start,&device,2,registration(&mut authenticator,&start));
    let key=device.public_key();
    let policy_gate=f.state.account_gate.lock().await;
    let mut attempt=Box::pin(f.service.handle(&f.state,&key,request.clone()));
    let receipt=tokio::select!{
        _=&mut attempt=>panic!("Finish returned before the held policy gate"),
        receipt=committed_finish_receipt(&f)=>receipt,
    };
    // Corrupt policy only after the signup and receipt are durable.
    std::fs::write(&settings,"invalid policy").unwrap();drop(policy_gate);
    assert_eq!(attempt.await.unwrap(),receipt);
    assert_eq!(f.service.handle(&f.state,&key,request.clone()).await.unwrap(),receipt);
    let handle=receipt["body"]["session"]["handle"].as_str().unwrap();
    assert_eq!(f.service.status(&f.state,&key,handle).await.unwrap()["body"]["state"],"pending");
    assert!(f.state.users.get_user_by_name("policyrecovery").await.unwrap().is_none());
    std::fs::write(&settings,r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
    let other=DeviceIdentity::from_seed([76;32]).unwrap().public_key();
    assert!(f.service.status(&f.state,&other,handle).await.is_err());
    assert!(f.service.handle(&f.state,&other,request.clone()).await.is_err());
    assert!(f.state.users.get_user_by_name("policyrecovery").await.unwrap().is_none());
    assert_eq!(f.service.status(&f.state,&key,handle).await.unwrap()["body"]["state"],"approved");
    assert_eq!(f.service.handle(&f.state,&key,request).await.unwrap(),receipt);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(f.db.pool()).await.unwrap(),1);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinPasskeys").fetch_one(f.db.pool()).await.unwrap(),1);
}

#[tokio::test]
async fn mesh_autoapproval_recovery_obeys_current_manual_disabled_and_denied_decisions(){
    for (current_policy,denied) in [
        (r#"{"allowSignups":true,"meshSignupApproval":"manual"}"#,false),
        (r#"{"allowSignups":false,"meshSignupApproval":"automatic"}"#,false),
        (r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#,true),
    ]{
        let f=Fixture::new().await;let device=DeviceIdentity::from_seed([77;32]).unwrap();
        let settings=f.directory.path().join("bluefin-accounts.json");
        std::fs::write(&settings,r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
        let start=begin(&f,f.invite(InviteIntent::Signup,None).await,&device,1,Some("reviewviewer")).await;
        let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
        let request=finish_request(&start,&device,2,registration(&mut authenticator,&start));
        // Force the real account owner's transaction to fail after Finish commits.
        sqlx::query("CREATE TRIGGER FailMeshAutoApproval BEFORE UPDATE ON BluefinMeshSignups WHEN NEW.State='approved' BEGIN SELECT RAISE(ABORT,'fixture approval failure'); END")
            .execute(f.db.writer()).await.unwrap();
        let key=device.public_key();
        let receipt=f.service.handle(&f.state,&key,request.clone()).await.unwrap();
        let handle=receipt["body"]["session"]["handle"].as_str().unwrap();
        assert!(f.state.users.get_user_by_name("reviewviewer").await.unwrap().is_none());
        sqlx::query("DROP TRIGGER FailMeshAutoApproval").execute(f.db.writer()).await.unwrap();
        std::fs::write(&settings,current_policy).unwrap();
        if denied{
            sqlx::query("UPDATE BluefinMeshSignups SET State='denied' WHERE Id=? AND State='pending'")
                .bind(handle).execute(f.db.writer()).await.unwrap();
        }
        assert_eq!(f.service.handle(&f.state,&key,request).await.unwrap(),receipt);
        let status=f.service.status(&f.state,&key,handle).await.unwrap();
        assert_eq!(status["body"]["state"],if denied{"denied"}else{"pending"});
        assert!(f.state.users.get_user_by_name("reviewviewer").await.unwrap().is_none());
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(f.db.pool()).await.unwrap(),1);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinPasskeys").fetch_one(f.db.pool()).await.unwrap(),0);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),0);
    }
}

#[tokio::test]
async fn mesh_new_epoch_fences_old_process_and_old_qr_but_retains_device_receipts(){
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let used_invite=f.invite(InviteIntent::Login,Some(user)).await;
    let unused=f.invite(InviteIntent::Login,Some(user)).await;
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let start=begin(&f,used_invite.clone(),&device,1,None).await;
    let request=finish_request(&start,&device,2,assertion(&mut authenticator,&start));
    let result=f.service.handle(&f.state,&device.public_key(),request.clone()).await.unwrap();
    let restarted=Service::new(f.db.clone(),"https://accounts.example",address()).await.unwrap();
    assert!(f.service.live().await.is_err());
    assert!(restarted.invite_row(&unused).await.is_err());
    assert!(restarted.invite_row(&used_invite).await.is_err());
    assert_eq!(restarted.handle(&f.state,&device.public_key(),request).await.unwrap(),result);
}

#[tokio::test]
async fn mesh_transport_real_pq_passkey_owner_router_and_no_portable_bearer(){
    use bluefin_mesh_connect::native::PqcEndpoint;
    use ferrofin_core::authorization_context::{FerrofinAuthorizationContext,FerrofinAuthService};
    use axum::{body::Body,http::Request as HttpRequest};
    use tower::ServiceExt;
    let mut f=Fixture::new().await;
    let server=PqcEndpoint::server(&DeviceIdentity::from_seed([31;32]).unwrap(),
        "127.0.0.1:0".parse().unwrap(),vec![],SERVICE).await.unwrap();
    let address=server.server_address().unwrap();
    f.service=Service::new(f.db.clone(),"https://accounts.example",address.clone()).await.unwrap();
    let mut inner=(*f.state).clone();
    let authorization=FerrofinAuthorizationContext::new(f.db.clone(),inner.users.clone(),
        inner.app_host.clone(),inner.config.clone(),"mesh-test-server","1");
    inner.auth_context=Arc::new(authorization.clone());
    inner.auth_service=Arc::new(FerrofinAuthService::new(authorization));
    f.state=AppState::from_inner(inner).with_mesh_accounts(f.service.clone());
    let (user,mut authenticator)=f.login_account().await;
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    let router=crate::create_router(f.state.clone());
    let (stop,stopping)=tokio::sync::watch::channel(false);
    let listener=tokio::spawn(crate::mesh_transport::run(server,f.state.clone(),router.clone(),stopping));
    let device=DeviceIdentity::from_seed([32;32]).unwrap();
    let client=PqcEndpoint::client(&device).await.unwrap();
    let mut connection=client.connect(&invite).await.unwrap();
    connection.send(&envelope(Request::Begin{request_id:[41;32],invitation:invite,
        device_public_key:device.public_key(),device_label:"Actual PQ peer".into(),account_name:None})).await.unwrap();
    let start:Value=connection.receive().await.unwrap();
    assert_eq!(start["body"]["type"],"begin_result");
    let finish=finish_request(&start,&device,42,assertion(&mut authenticator,&start));
    connection.send(&finish).await.unwrap();
    let result:Value=connection.receive().await.unwrap();
    let session=result["body"]["session"]["id"].as_str().unwrap();
    let device_id:String=sqlx::query_scalar("SELECT DeviceId FROM BluefinMeshSessions WHERE Id=?")
        .bind(session).fetch_one(f.db.pool()).await.unwrap();
    f.sessions.add("wire-playback", user, &device_id);
    // This synthetic server-only Devices row is test setup for the existing
    // auth context. The invitation, passkey, PQ channel and grant are real.
    let token="synthetic-mesh-private-bearer-for-test-only";
    let now=chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO Devices(AccessToken,AppName,AppVersion,DateCreated,DateLastActivity,DateModified,DeviceId,DeviceName,IsActive,UserId) VALUES(?,?,?,?,?,?,?,?,1,?)")
        .bind(token).bind("Bluefin").bind("1").bind(&now).bind(&now).bind(&now)
        .bind(&device_id).bind("Actual PQ peer").bind(guid_to_db(user)).execute(f.db.writer()).await.unwrap();
    let request=json!({"version":1,"body":{"type":"control","session_id":session,
        "binding":connection.binding(),"request":{"method":"GET","path":"/Users/Me","headers":[],"body":[]}}});
    connection.send(&request).await.unwrap();
    let head:Value=connection.receive().await.unwrap();
    assert_eq!(head["body"]["type"],"response_head");assert_eq!(head["body"]["status"],200);
    let mut body=Vec::new();
    loop{
        let frame:Value=connection.receive().await.unwrap();
        if frame["body"]["type"]=="response_end"{break;}
        assert_eq!(frame["body"]["type"],"response_chunk");
        body.extend(serde_json::from_value::<Vec<u8>>(frame["body"]["bytes"].clone()).unwrap());
    }
    let viewer:Value=serde_json::from_slice(&body).unwrap();assert_eq!(viewer["Name"],"viewer");
    assert!(!String::from_utf8(body).unwrap().contains(token));
    let http=router.oneshot(HttpRequest::builder().uri("/Users/Me")
        .header("Authorization",format!("MediaBrowser Token=\"{token}\", Device=\"Bluefin\""))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(http.status(),401,"server-only mesh bearer must not authenticate ordinary HTTP");
    let mut wrong_binding=request.clone();
    let mut binding=connection.binding();binding[0]^=1;wrong_binding["body"]["binding"]=json!(binding);
    connection.send(&wrong_binding).await.unwrap();
    assert!(connection.receive::<Value>().await.is_err());
    let thief=PqcEndpoint::client(&DeviceIdentity::from_seed([33;32]).unwrap()).await.unwrap();
    let mut other=thief.connect_server(&address).await.unwrap();
    let mut stolen=request;stolen["body"]["binding"]=json!(other.binding());
    other.send(&stolen).await.unwrap();
    assert!(other.receive::<Value>().await.is_err(),"a copied public session descriptor grants nothing");
    other.close().await;connection.close().await;
    let mut revoking = client.connect_server(&address).await.unwrap();
    let revoke_id = [77u8;32];
    let revoke = json!({"version":1,"body":{"type":"revoke_session","session_id":session,
        "request_id":revoke_id,"binding":revoking.binding()}});
    let mut wrong_revoke = revoke.clone();
    let mut invalid = revoking.binding(); invalid[0] ^= 1;
    wrong_revoke["body"]["binding"] = json!(invalid);
    revoking.send(&wrong_revoke).await.unwrap();
    assert!(revoking.receive::<Value>().await.is_err());
    revoking.close().await;
    let mut revoking = client.connect_server(&address).await.unwrap();
    let mut revoke = revoke; revoke["body"]["binding"] = json!(revoking.binding());
    revoking.send(&revoke).await.unwrap();
    let revoked: Value = revoking.receive().await.unwrap();
    assert_eq!(revoked["body"]["type"], "session_revoked");
    assert_eq!(revoked["body"]["request_id"], json!(revoke_id));
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM Devices WHERE DeviceId=?")
        .bind(&device_id).fetch_one(f.db.pool()).await.unwrap(), 0);
    revoking.close().await;
    let mut retry = client.connect_server(&address).await.unwrap();
    revoke["body"]["binding"] = json!(retry.binding());
    retry.send(&revoke).await.unwrap();
    assert_eq!(retry.receive::<Value>().await.unwrap(), revoked);
    retry.close().await;
    assert!(!f.sessions.contains("wire-playback"));
    assert_eq!(f.sessions.close_attempts(), ["wire-playback"]);
    client.close().await;thief.close().await;
    stop.send(true).unwrap();let receipt=listener.await.unwrap().unwrap();
    assert!(receipt.endpoint_closed);assert!(receipt.joined_connections>=2);
}

#[tokio::test]
async fn mesh_browser_capability_is_single_use_and_callback_alone_cannot_finish(){
    use tower::ServiceExt;
    use axum::{body::Body,http::Request as HttpRequest};
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let device=DeviceIdentity::from_seed([7;32]).unwrap();
    let start=begin(&f,f.invite(InviteIntent::Login,Some(user)).await,&device,1,None).await;
    let request=finish_request(&start,&device,2,json!({"browser":true}));
    // An early/forged callback does not create a session or discard the claim.
    assert!(f.service.handle(&f.state,&device.public_key(),request.clone()).await.is_err());
    let url=Url::parse(start["body"]["passkey_options"]["browserUrl"].as_str().unwrap()).unwrap();
    let (challenge,secret)=url.fragment().unwrap().split_once('.').unwrap();
    let body=json!({"challenge":challenge,"secret":secret,"credential":assertion(&mut authenticator,&start)});
    let router=web::routes().with_state(f.state.clone().with_mesh_accounts(f.service.clone()));
    let post=||HttpRequest::builder().method("POST").uri("/Bluefin/Accounts/Mesh/Passkey/Verify")
        .header("content-type","application/json").header("origin","https://accounts.example")
        .body(Body::from(serde_json::to_vec(&body).unwrap())).unwrap();
    assert_eq!(router.clone().oneshot(post()).await.unwrap().status(),axum::http::StatusCode::NO_CONTENT);
    assert!(!router.oneshot(post()).await.unwrap().status().is_success());
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),0);
    let completed=f.service.handle(&f.state,&device.public_key(),request).await.unwrap();
    assert_eq!(completed["body"]["session"]["kind"],"session");
}
#[tokio::test]
async fn mesh_signup_uses_current_approval_policy_at_finish(){
    let f=Fixture::new().await;let device=DeviceIdentity::from_seed([7;32]).unwrap();
    std::fs::write(f.directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"automatic"}"#).unwrap();
    let invite=f.invite(InviteIntent::Signup,None).await;
    let start=begin(&f,invite,&device,1,Some("policyviewer")).await;
    std::fs::write(f.directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"manual"}"#).unwrap();
    let mut authenticator=WebauthnAuthenticator::new(SoftPasskey::new(true));
    let request=finish_request(&start,&device,2,registration(&mut authenticator,&start));
    let completed=f.service.handle(&f.state,&device.public_key(),request).await.unwrap();
    assert_eq!(completed["body"]["session"]["state"],"pending");
    assert!(f.state.users.get_user_by_name("policyviewer").await.unwrap().is_none());
}

async fn signed_in_mesh_session(f: &Fixture, device: &DeviceIdentity) -> String {
    let (user, mut authenticator) = f.login_account().await;
    let start = begin(f, f.invite(InviteIntent::Login, Some(user)).await, device, 81, None).await;
    let finish = finish_request(&start, device, 82, assertion(&mut authenticator, &start));
    let session = f.service.handle(&f.state, &device.public_key(), finish).await.unwrap()
        ["body"]["session"]["id"].as_str().unwrap().to_owned();
    let device_id: String = sqlx::query_scalar("SELECT DeviceId FROM BluefinMeshSessions WHERE Id=?")
        .bind(&session).fetch_one(f.db.pool()).await.unwrap();
    f.sessions.add("target-playback", user, &device_id);
    session
}

async fn mesh_test_bearer(f: &Fixture, session: &str) {
    let row = sqlx::query("SELECT UserId,DeviceId FROM BluefinMeshSessions WHERE Id=?")
        .bind(session).fetch_one(f.db.pool()).await.unwrap();
    let stamp = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO Devices(AccessToken,AppName,AppVersion,DateCreated,DateLastActivity,DateModified,DeviceId,DeviceName,IsActive,UserId) VALUES(?,?,?,?,?,?,?,?,1,?)")
        .bind("synthetic-revocation-bearer-only").bind("Bluefin").bind("1")
        .bind(&stamp).bind(&stamp).bind(&stamp)
        .bind(row.get::<String, _>("DeviceId")).bind("Revoke test")
        .bind(row.get::<String, _>("UserId")).execute(f.db.writer()).await.unwrap();
}

#[tokio::test]
async fn mesh_revoke_requires_original_device_and_retains_one_exact_receipt() {
    let f = Fixture::new().await;
    let device = DeviceIdentity::from_seed([71;32]).unwrap();
    let session = signed_in_mesh_session(&f, &device).await;
    let key = device.public_key();
    let other = DeviceIdentity::from_seed([72;32]).unwrap().public_key();
    assert!(f.service.revoke_session(&f.state, &other, &session, [83;32]).await.is_err());
    assert!(f.sessions.close_attempts().is_empty());
    let (first, retry) = tokio::join!(
        f.service.revoke_session(&f.state, &key, &session, [83;32]),
        f.service.revoke_session(&f.state, &key, &session, [83;32]),
    );
    let receipt = first.unwrap();
    assert_eq!(receipt, retry.unwrap());
    assert_eq!(receipt["body"]["type"], "session_revoked");
    assert!(!f.sessions.contains("target-playback"));
    assert_eq!(f.sessions.close_attempts(), ["target-playback"]);
    assert!(f.service.revoke_session(&f.state, &key, &session, [84;32]).await.is_err());
    assert!(f.service.revoke_session(&f.state, &other, &session, [83;32]).await.is_err());
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM BluefinMeshSessionRevocations")
        .fetch_one(f.db.pool()).await.unwrap(), 1);
    assert!(sqlx::query("UPDATE BluefinMeshSessions SET Revoked=0 WHERE Id=?")
        .bind(&session).execute(f.db.writer()).await.is_err());
    assert!(sqlx::query("UPDATE BluefinMeshSessionRevocations SET Response='{}' WHERE SessionId=?")
        .bind(&session).execute(f.db.writer()).await.is_err());
}

#[tokio::test]
async fn mesh_revoke_survives_cleanup_failure_and_exact_retry_after_owner_restart() {
    let f = Fixture::new().await;
    let device = DeviceIdentity::from_seed([73;32]).unwrap();
    let session = signed_in_mesh_session(&f, &device).await;
    mesh_test_bearer(&f, &session).await;
    let row = sqlx::query("SELECT UserId,DeviceId FROM BluefinMeshSessions WHERE Id=?")
        .bind(&session).fetch_one(f.db.pool()).await.unwrap();
    let user = Uuid::parse_str(&row.get::<String, _>("UserId")).unwrap();
    f.sessions.add("other-device", user, "different-device");
    f.sessions.add("other-account", Uuid::new_v4(), &row.get::<String, _>("DeviceId"));
    assert!(f.service.authorization(&f.state, &device.public_key(), &session, "test").await.is_ok());

    f.sessions.fail_next_close();
    assert!(f.service.revoke_session(&f.state, &device.public_key(), &session, [85;32]).await.is_err());
    // The security mutation is durable even though cleanup prevented an ACK.
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM Devices")
        .fetch_one(f.db.pool()).await.unwrap(), 0);
    assert!(f.service.authorization(&f.state, &device.public_key(), &session, "test").await.is_err());
    let saved: String = sqlx::query_scalar("SELECT Response FROM BluefinMeshSessionRevocations WHERE SessionId=?")
        .bind(&session).fetch_one(f.db.pool()).await.unwrap();
    let response: Value = serde_json::from_str(&saved).unwrap();
    assert!(f.sessions.contains("target-playback"));
    assert_eq!(f.sessions.close_attempts(), ["target-playback"]);

    let restarted = Service::new(f.db.clone(), "https://accounts.example", address()).await.unwrap();
    assert!(f.service.revoke_session(&f.state, &device.public_key(), &session, [85;32]).await.is_err());
    assert_eq!(restarted.revoke_session(&f.state, &device.public_key(), &session, [85;32]).await.unwrap(), response);
    assert!(!f.sessions.contains("target-playback"));
    assert!(f.sessions.contains("other-device"));
    assert!(f.sessions.contains("other-account"));
    assert_eq!(f.sessions.close_attempts(), ["target-playback", "target-playback"]);
    assert_eq!(restarted.revoke_session(&f.state, &device.public_key(), &session, [85;32]).await.unwrap(), response);
    assert_eq!(f.sessions.close_attempts(), ["target-playback", "target-playback"]);
    assert!(restarted.authorization(&f.state, &device.public_key(), &session, "test").await.is_err());
}

#[tokio::test]
async fn mesh_existing_bearer_deletion_terminally_revokes_the_device_grant() {
    let f = Fixture::new().await;
    let device = DeviceIdentity::from_seed([74;32]).unwrap();
    let session = signed_in_mesh_session(&f, &device).await;
    mesh_test_bearer(&f, &session).await;
    // SessionManager.logout deletes this Devices row through its existing owner.
    sqlx::query("DELETE FROM Devices WHERE AccessToken=?")
        .bind("synthetic-revocation-bearer-only").execute(f.db.writer()).await.unwrap();
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT Revoked FROM BluefinMeshSessions WHERE Id=?")
        .bind(&session).fetch_one(f.db.pool()).await.unwrap(), 1);
    assert!(f.service.authorization(&f.state, &device.public_key(), &session, "test").await.is_err());
    // A later explicit request can retain its receipt after ordinary logout.
    assert!(f.service.revoke_session(&f.state, &device.public_key(), &session, [86;32]).await.is_ok());
    assert!(!f.sessions.contains("target-playback"));
    assert_eq!(f.sessions.close_attempts(), ["target-playback"]);
}

#[tokio::test]
async fn mesh_web_native_and_rendered_qr_share_one_live_invitation() {
    use base64::Engine;
    let mut f=Fixture::new().await;
    f.service=Service::new_with_web_launch(f.db.clone(), "https://accounts.example",
        address(), Some("https://app.example/web/bluefin/")).await.unwrap();
    let (user,mut authenticator)=f.login_account().await;
    let issued=f.service.issue(&f.state,InviteIntent::Login,Some(user),600).await.unwrap();
    let original=ConnectInvite::parse(issued["code"].as_str().unwrap()).unwrap();
    for field in ["link","webLink","nativeLink"] {
        assert_eq!(ConnectInvite::parse(issued[field].as_str().unwrap()).unwrap(),original);
    }
    let url=Url::parse(issued["webLink"].as_str().unwrap()).unwrap();
    assert_eq!(url.origin().ascii_serialization(),"https://app.example");
    assert_eq!(url.path(),"/web/bluefin/");
    assert!(url.query().is_none());
    assert!(url.fragment().unwrap().starts_with("bluefin-connect="));
    let png=base64::engine::general_purpose::STANDARD.decode(issued["pngBase64"].as_str().unwrap()).unwrap();
    assert_eq!(bluefin_mesh_connect::decode_qr_image(&png).unwrap(),original);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshInvites")
        .fetch_one(f.db.pool()).await.unwrap(),1);
    let device=DeviceIdentity::from_seed([90;32]).unwrap();
    let first=begin(&f,ConnectInvite::parse(issued["webLink"].as_str().unwrap()).unwrap(),&device,91,None).await;
    assert_eq!(first["body"]["passkey_options"]["origin"], "https://accounts.example");
    assert_eq!(first["body"]["passkey_options"]["rpId"], "accounts.example");
    let ceremony=Url::parse(first["body"]["passkey_options"]["browserUrl"].as_str().unwrap()).unwrap();
    assert_eq!(ceremony.origin().ascii_serialization(), "https://accounts.example");
    let other=DeviceIdentity::from_seed([92;32]).unwrap();
    let reused=envelope(Request::Begin{request_id:[93;32],
        invitation:ConnectInvite::parse(issued["nativeLink"].as_str().unwrap()).unwrap(),
        device_public_key:other.public_key(),device_label:"Second representation".into(),account_name:None});
    assert!(f.service.handle(&f.state,&other.public_key(),reused).await.is_err());
    let finish=finish_request(&first,&device,94,assertion(&mut authenticator,&first));
    assert!(f.service.handle(&f.state,&device.public_key(),finish).await.is_ok());
}


#[tokio::test]
async fn mesh_invalid_web_launch_cannot_retire_the_live_issuer() {
    let f=Fixture::new().await;
    for url in ["http://app.example/", "https://user@app.example/",
        "https://app.example/?code=secret", "https://app.example/#secret", "https://192.0.2.1/"] {
        assert!(Service::new_with_web_launch(f.db.clone(), "https://accounts.example",
            address(), Some(url)).await.is_err());
        assert!(f.service.live().await.is_ok());
    }
}
