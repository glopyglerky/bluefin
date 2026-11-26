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
