//! Exercises real passkey and ML-DSA proofs against the production SQL owner.
use super::*;
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
struct Fixture { directory:tempfile::TempDir, state:AppState, service:Arc<Service>, db:Database }
impl Fixture {
    async fn new()->Self {
        let directory=tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("bluefin-accounts.json"),r#"{"allowSignups":true,"meshSignupApproval":"manual"}"#).unwrap();
        let db=Database::connect_in_memory().await.unwrap();db.run_migrations().await.unwrap();
        let mut inner=(*crate::test_support::fake_state()).clone();
        inner.users=Arc::new(FerrofinUserManager::new(db.clone()));
        inner.config=Arc::new(Config(Arc::new(Paths(directory.path().to_string_lossy().into_owned()))));
        let state=AppState::from_inner(inner);
        let service=Service::new(db.clone(),"https://accounts.example",address()).await.unwrap();
        Self{directory,state,service,db}
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
async fn mesh_login_consumes_once_across_devices_and_only_exact_device_retry_recovers(){
    let f=Fixture::new().await;let (user,mut authenticator)=f.login_account().await;
    let invite=f.invite(InviteIntent::Login,Some(user)).await;
    let first=DeviceIdentity::from_seed([7;32]).unwrap();
    let second=DeviceIdentity::from_seed([8;32]).unwrap();
    let a=begin(&f,invite.clone(),&first,1,None).await;
    let b=begin(&f,invite.clone(),&second,2,None).await;
    let ra=finish_request(&a,&first,3,assertion(&mut authenticator,&a));
    let rb=finish_request(&b,&second,4,assertion(&mut authenticator,&b));
    let first_key=first.public_key();let second_key=second.public_key();
    let (a,b)=tokio::join!(
        f.service.handle(&f.state,&first_key,ra.clone()),
        f.service.handle(&f.state,&second_key,rb.clone())
    );
    assert_ne!(a.is_ok(),b.is_ok(),"only one proof may spend a code");
    let (key,request,result,other)=if let Ok(a)=a {(first_key,ra,a,second_key)}else{(second_key,rb,b.unwrap(),first_key)};
    assert_eq!(f.service.handle(&f.state,&key,request.clone()).await.unwrap(),result);
    assert!(f.service.handle(&f.state,&other,request.clone()).await.is_err());
    let mut changed=request;changed["body"]["device_signature"][0]=json!(0);
    assert!(f.service.handle(&f.state,&key,changed).await.is_err());
    assert!(!serde_json::to_string(&result).unwrap().contains("AccessToken"));
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM BluefinMeshSessions").fetch_one(f.db.pool()).await.unwrap(),1);
    assert!(f.service.invite_row(&invite).await.is_err());
    assert!(sqlx::query("UPDATE BluefinMeshInvites SET State='issued'").execute(f.db.writer()).await.is_err());
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
    client.close().await;thief.close().await;
    stop.send(true).unwrap();let receipt=listener.await.unwrap().unwrap();
    assert!(receipt.endpoint_closed);assert!(receipt.joined_connections>=2);
}
