//! Device-bound onboarding through Bluefin's existing account owner.
//! QR images and text share one invitation. A successful passkey proof consumes
//! it atomically with the grant and receipt. Activation replaces a live epoch,
//! so restoring an older database cannot make an old code usable.
use std::{collections::HashMap, sync::Arc};
use axum::{Json, Router, extract::{Path, State}, routing::{get, post}};
use bluefin_mesh_connect::{ConnectInvite, InviteIntent, ServerAddress, QrTheme, qr_png_for_web, qr_svg_for_web, launch::WebLaunchBase};
use bluefin_mesh_connect::native::verify_device_signature;
use bluefin_mesh_connect::protocol::{Envelope, Request, Response};
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_model::secret::Secret;
use ferrofin_traits::{error::ServiceError, session::AuthenticationRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tokio::sync::{Mutex, Semaphore};
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::*;
use crate::{auth::{RequireAdmin, RequireAuth, is_administrator_identity}, error::ApiError, state::AppState};

pub const SERVICE: &str = "bluefin/connect/1";
const MAX_RECORDS: i64 = 16_384;
const MAX_PENDING: usize = 128;
const SESSION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
fn failed() -> ApiError { ApiError::Unauthorized("This connection could not be verified.".into()) }
fn used() -> ApiError { ApiError::Conflict("This connection code is expired or already used. Ask for a new code.".into()) }
fn storage(_: impl std::fmt::Display) -> ApiError { ServiceError::backend("Mesh account storage is unavailable.").into() }
fn now() -> i64 { chrono::Utc::now().timestamp_millis() }
fn digest(bytes: &[u8]) -> [u8;32] { Sha256::digest(bytes).into() }
fn random() -> Result<[u8;32],ApiError> { let mut bytes=[0;32]; getrandom::fill(&mut bytes).map_err(storage)?; Ok(bytes) }
fn hex(bytes:&[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len()*2), |mut s,b| {let _=write!(s,"{b:02x}");s})
}
fn id_valid(id:&str)->bool {id.len()==64 && id.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c))}
fn name_valid(name:&str)->bool { !name.is_empty() && name.len()<=64 && name.trim()==name && !name.chars().any(char::is_control) }

/// Internal server authorization. Never serialize it into a mesh response.
pub struct MeshAuthorization { pub token: Secret, pub device_id: String }
enum Ceremony { Login { user:Uuid, state:PasskeyAuthentication }, Signup { username:String, state:PasskeyRegistration } }
struct Pending {
    begin_id:[u8;32], begin_hash:[u8;32], invite:ConnectInvite, peer:Vec<u8>,
    label:String, expires:i64, proof:String, ceremony:Ceremony, response:Value,
    browser_secret_hash:[u8;32], browser_response:Option<Value>, browser_used:bool,
    finish_attempt:Option<([u8;32],[u8;32])>,
}
/// This is an adapter over the account database, not another identity store.
pub struct Service {
    db:Database, address:ServerAddress, authority:[u8;32], epoch:[u8;32], generation:u64,
    webauthn:Webauthn, rp_id:String, origin:String, browser_page:Url, web_launch:WebLaunchBase,
    pending:Mutex<HashMap<String,Pending>>, gate:Mutex<()>, operations:Semaphore,
}
impl Service {
    /// Activates the issuer; an older process fails its next epoch comparison.
    pub async fn new(db:Database, published_url:&str, address:ServerAddress)->Result<Arc<Self>,ApiError> {
        Self::new_with_web_launch(db, published_url, address, None).await
    }
    /// The operator may host the launch page separately. This does not change
    /// the passkey relying party, allowed ceremony origin or account API.
    pub async fn new_with_web_launch(
        db:Database, published_url:&str, address:ServerAddress, web_launch_url:Option<&str>,
    )->Result<Arc<Self>,ApiError> {
        if address.service!=SERVICE { return Err(failed()); }
        let origin_url=Url::parse(published_url).map_err(|_|failed())?;
        if origin_url.scheme()!="https" || !origin_url.username().is_empty() || origin_url.password().is_some()
            || origin_url.query().is_some() || origin_url.fragment().is_some() {return Err(failed());}
        let rp_id=origin_url.domain().ok_or_else(failed)?.to_owned();
        let origin=origin_url.origin().ascii_serialization();
        let webauthn=WebauthnBuilder::new(&rp_id,&Url::parse(&origin).map_err(|_|failed())?)
            .map_err(|_|failed())?.rp_name("Bluefin").build().map_err(|_|failed())?;
        let mut browser_page=origin_url.clone();
        browser_page.path_segments_mut().map_err(|_|failed())?.pop_if_empty().extend(["Bluefin","Accounts","Mesh","Passkey"]);
        let default_launch=format!("{origin}/web/bluefin/");
        // Validate before activating a new epoch: a bad launch configuration
        // must not retire the existing live issuer.
        let web_launch=WebLaunchBase::parse(web_launch_url.unwrap_or(&default_launch)).map_err(|_|failed())?;
        let epoch=random()?;
        let mut tx=db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        sqlx::query("INSERT INTO BluefinMeshAuthority(Id,AuthorityId,LiveEpoch,Generation) VALUES(1,?,?,1) ON CONFLICT(Id) DO NOTHING")
            .bind(random()?.to_vec()).bind(epoch.to_vec()).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE BluefinMeshAuthority SET LiveEpoch=? WHERE Id=1").bind(epoch.to_vec()).execute(&mut *tx).await.map_err(storage)?;
        sqlx::query("UPDATE BluefinMeshInvites SET State='revoked' WHERE State IN ('issued','claimed') AND Epoch!=?")
            .bind(epoch.to_vec()).execute(&mut *tx).await.map_err(storage)?;
        let row=sqlx::query("SELECT AuthorityId,Generation FROM BluefinMeshAuthority WHERE Id=1").fetch_one(&mut *tx).await.map_err(storage)?;
        let authority:Vec<u8>=row.get("AuthorityId");
        let authority=authority.try_into().map_err(|_|failed())?;
        let generation=u64::try_from(row.get::<i64,_>("Generation")).map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        Ok(Arc::new(Self {db,address,authority,epoch,generation,webauthn,rp_id,origin,browser_page,web_launch,
            pending:Mutex::new(HashMap::new()),gate:Mutex::new(()),operations:Semaphore::new(16)}))
    }
    pub fn address(&self)->&ServerAddress {&self.address}
    async fn live(&self)->Result<(),ApiError> {
        let current:Option<Vec<u8>>=sqlx::query_scalar("SELECT LiveEpoch FROM BluefinMeshAuthority WHERE Id=1")
            .fetch_optional(self.db.pool()).await.map_err(storage)?;
        if current.as_deref()!=Some(self.epoch.as_slice()) {return Err(used());} Ok(())
    }
    async fn live_tx(&self,tx:&mut sqlx::Transaction<'_,sqlx::Sqlite>)->Result<(),ApiError>{
        let current:Option<Vec<u8>>=sqlx::query_scalar("SELECT LiveEpoch FROM BluefinMeshAuthority WHERE Id=1")
            .fetch_optional(&mut **tx).await.map_err(storage)?;
        if current.as_deref()!=Some(self.epoch.as_slice()) {return Err(used());} Ok(())
    }
    async fn policy(&self,state:&AppState)->Result<super::bluefin_accounts::Settings,ApiError>{
        if !state.config.configuration().await?.is_startup_wizard_completed {return Err(failed());}
        super::bluefin_accounts::read_settings(state).await
    }
    async fn credentials(&self,user:Uuid)->Result<Vec<Passkey>,ApiError>{
        let rows:Vec<String>=sqlx::query_scalar("SELECT Credential FROM BluefinPasskeys WHERE UserId=? ORDER BY CredentialId LIMIT 12")
            .bind(guid_to_db(user)).fetch_all(self.db.pool()).await.map_err(storage)?;
        rows.iter().map(|s|serde_json::from_str(s).map_err(storage)).collect()
    }
    async fn account_allowed(&self,state:&AppState,user:Uuid)->Result<(),ApiError>{
        let user=state.users.get_user_by_id(user).await?.ok_or_else(failed)?;
        let policy=state.users.get_user_dto(&user,None).await?.policy.ok_or_else(failed)?;
        // Mesh is a remote origin even when its current network hop is on a LAN.
        if policy.is_disabled || !policy.enable_remote_access {return Err(failed());}
        Ok(())
    }
    pub async fn issue(&self,state:&AppState,intent:InviteIntent,user:Option<Uuid>,lifetime:u64)->Result<Value,ApiError>{
        let _gate=self.gate.lock().await;
        self.live().await?;
        let policy=self.policy(state).await?;
        match intent {
            InviteIntent::Login => {
                let user=user.ok_or_else(failed)?; self.account_allowed(state,user).await?;
                if self.credentials(user).await?.is_empty() { return Err(ApiError::Conflict("Add a passkey before creating a connection code.".into())); }
            },
            InviteIntent::Signup => if !policy.allow_signups || user.is_some() {return Err(failed());},
        }
        if !(30..=3600).contains(&lifetime) {return Err(ApiError::BadRequest("Choose a code lifetime between 30 and 3600 seconds.".into()));}
        let hint=user.map(|id| {let mut bytes=self.authority.to_vec();bytes.extend_from_slice(id.as_bytes());digest(&bytes)});
        let invite=ConnectInvite {
            authority_id:self.authority,authority_generation:self.generation,authority_epoch:self.epoch,
            endpoint_id:self.address.endpoint_id,server_certificate_sha256:self.address.server_certificate_sha256,
            invitation_id:random()?,secret:random()?,expires_at_unix_ms:(now() as u64)+lifetime*1000,
            service:SERVICE.into(),intent,account_hint:hint,
            relay_urls:self.address.relay_urls.clone(),direct_addresses:self.address.direct_addresses.clone(),
        };
        let code=invite.to_code().map_err(|_|failed())?;
        let web_link=self.web_launch.invitation_url(&invite).map_err(|_|failed())?;
        let native_link=format!("bluefin://connect/{code}");
        let theme=QrTheme::bluefin();
        // Prepare every representation before committing an invitation that
        // could otherwise be stranded by an image/URL size failure.
        use base64::Engine;
        let png=base64::engine::general_purpose::STANDARD
            .encode(qr_png_for_web(&invite,&self.web_launch,&theme).map_err(|_|failed())?);
        let svg=qr_svg_for_web(&invite,&self.web_launch,&theme).map_err(|_|failed())?;
        let mut tx=self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        self.live_tx(&mut tx).await?;
        let changed=sqlx::query("INSERT INTO BluefinMeshInvites(Id,Epoch,CapsuleHash,Intent,UserId,Expires,State) SELECT ?,?,?,?,?,?,'issued' WHERE (SELECT COUNT(*) FROM BluefinMeshInvites)<?")
            .bind(invite.invitation_id.to_vec()).bind(self.epoch.to_vec()).bind(digest(code.as_bytes()).to_vec())
            .bind(match intent {InviteIntent::Login=>"login",InviteIntent::Signup=>"signup"})
            .bind(user.map(guid_to_db)).bind(invite.expires_at_unix_ms as i64).bind(MAX_RECORDS)
            .execute(&mut *tx).await.map_err(storage)?.rows_affected();
        if changed!=1 {return Err(ApiError::ServiceUnavailable("The connection receipt store is full.".into()));}
        tx.commit().await.map_err(storage)?;
        Ok(json!({"code":code,"link":web_link,"webLink":web_link,"nativeLink":native_link,
            "pngBase64":png,"svg":svg,"expiresAtUnixMs":invite.expires_at_unix_ms,
            "intent":match intent {InviteIntent::Login=>"login",InviteIntent::Signup=>"signup"},"singleUse":true,"handle":hex(&invite.invitation_id)}))
    }
    async fn invite_row(&self,invite:&ConnectInvite)->Result<sqlx::sqlite::SqliteRow,ApiError>{
        invite.validate_at(now() as u64).map_err(|_|used())?;
        if invite.service!=SERVICE || invite.authority_id!=self.authority || invite.authority_generation!=self.generation
            || invite.authority_epoch!=self.epoch || invite.endpoint_id!=self.address.endpoint_id
            || invite.server_certificate_sha256!=self.address.server_certificate_sha256 {return Err(used());}
        let code=invite.to_code().map_err(|_|used())?;
        sqlx::query("SELECT UserId,State,ClaimedDevice,ClaimRequestId,ClaimRequestHash FROM BluefinMeshInvites WHERE Id=? AND Epoch=? AND CapsuleHash=? AND State IN ('issued','claimed') AND Expires>?")
            .bind(invite.invitation_id.to_vec()).bind(self.epoch.to_vec()).bind(digest(code.as_bytes()).to_vec()).bind(now())
            .fetch_optional(self.db.pool()).await.map_err(storage)?.ok_or_else(used)
    }
    /// The transport supplies the verified ML-DSA peer, after network admission.
    pub async fn handle(&self,state:&AppState,peer:&[u8],input:Value)->Result<Value,ApiError>{
        if peer.len()!=1952 {return Err(failed());}
        let _permit=self.operations.try_acquire().map_err(|_|ApiError::ServiceUnavailable("Please retry shortly.".into()))?;
        if input["body"]["type"]=="status" {
            let request:Envelope<Status>=serde_json::from_value(input).map_err(|_|failed())?;
            request.require_version().map_err(|_|failed())?;
            let _gate=self.gate.lock().await;
            return self.status(state,peer,&request.body.handle).await;
        }
        let envelope:Envelope<Request>=serde_json::from_value(input).map_err(|_|failed())?;
        envelope.require_version().map_err(|_|failed())?;
        envelope.body.validate().map_err(|_|failed())?;
        let request_hash=digest(&serde_json::to_vec(&envelope.body).map_err(storage)?);
        let request_id=envelope.body.request_id();
        let _gate=self.gate.lock().await;
        self.live().await?;
        if let Some(row)=sqlx::query("SELECT DevicePublicKey,RequestHash,Response FROM BluefinMeshReceipts WHERE RequestId=?")
            .bind(request_id.to_vec()).fetch_optional(self.db.pool()).await.map_err(storage)? {
            if row.get::<Vec<u8>,_>("DevicePublicKey")!=peer || row.get::<Vec<u8>,_>("RequestHash")!=request_hash {return Err(failed());}
            let saved:Value=serde_json::from_str(&row.get::<String,_>("Response")).map_err(storage)?;
            if saved["body"]["session"]["kind"]=="session" {
                let id=saved["body"]["session"]["id"].as_str().ok_or_else(failed)?;
                let active:Option<String>=sqlx::query_scalar("SELECT UserId FROM BluefinMeshSessions WHERE Id=? AND DevicePublicKey=? AND Revoked=0 AND Expires>?")
                    .bind(id).bind(peer).bind(now()).fetch_optional(self.db.pool()).await.map_err(storage)?;
                let user=Uuid::parse_str(&active.ok_or_else(failed)?).map_err(|_|failed())?;
                self.account_allowed(state,user).await?;
            } else if saved["body"]["session"]["kind"]=="signup" {
                let handle=saved["body"]["session"]["handle"].as_str().filter(|id|id_valid(id)).ok_or_else(failed)?;
                // The immutable receipt survives interrupted optional provisioning.
                // Only this same verified device may resume its pending signup.
                let _=self.resume_automatic_signup(state,peer,handle).await;
            }
            return Ok(saved);
        }
        match envelope.body {
            Request::Begin {request_id,invitation,device_public_key,device_label,account_name}=>{
                if device_public_key!=peer {return Err(failed());}
                self.begin(state,peer,request_id,request_hash,invitation,device_label,account_name).await
            },
            Request::Finish {request_id,challenge_id,passkey_response,device_signature}=>{
                self.finish(state,peer,request_id,request_hash,&challenge_id,passkey_response,&device_signature).await
            },
        }
    }
    async fn begin(&self,state:&AppState,peer:&[u8],request_id:[u8;32],request_hash:[u8;32],invite:ConnectInvite,label:String,name:Option<String>)->Result<Value,ApiError>{
        let mut pending=self.pending.lock().await;
        pending.retain(|_,value|value.expires>now());
        if let Some(previous)=pending.values().find(|v|v.begin_id==request_id) {
            return if previous.begin_hash==request_hash && previous.peer==peer {Ok(previous.response.clone())} else {Err(failed())};
        }
        if pending.len()>=MAX_PENDING || pending.values().filter(|v|v.invite.invitation_id==invite.invitation_id).count()>=4 {
            return Err(ApiError::ServiceUnavailable("Too many active connection requests.".into()));
        }
        let row=self.invite_row(&invite).await?;
        if row.get::<String,_>("State")!="issued" {return Err(used());}
        let (public,ceremony,kind)=match invite.intent {
            InviteIntent::Login=>{
                if name.is_some(){return Err(failed());}
                let id:Option<String>=row.get("UserId");
                let user=Uuid::parse_str(&id.ok_or_else(failed)?).map_err(|_|failed())?;
                self.account_allowed(state,user).await?;
                let keys=self.credentials(user).await?;
                let (public,private)=self.webauthn.start_passkey_authentication(&keys).map_err(|_|failed())?;
                (serde_json::to_value(public).map_err(storage)?,Ceremony::Login{user,state:private},"assertion")
            },
            InviteIntent::Signup=>{
                if !self.policy(state).await?.allow_signups {return Err(failed());}
                let credential_user=Uuid::new_v4();
                let (name,display)=match name {
                    Some(name) if name_valid(&name)=>(name.clone(),name),
                    Some(_)=>return Err(ApiError::BadRequest("Choose an account name of up to 64 bytes.".into())),
                    None=>(format!("bluefin-{}",credential_user.simple()),"Bluefin account".to_owned()),
                };
                if state.users.get_user_by_name(&name).await?.is_some(){return Err(ApiError::Conflict("This account name is unavailable.".into()));}
                let (public,private)=self.webauthn.start_passkey_registration(credential_user,&name,&display,None).map_err(|_|failed())?;
                (serde_json::to_value(public).map_err(storage)?,Ceremony::Signup{username:name,state:private},"registration")
            },
        };
        let challenge=hex(&random()?);
        let browser_secret=hex(&random()?);
        let browser_url=format!("{}#{}.{}",self.browser_page,challenge,browser_secret);
        let expires=std::cmp::min(now()+300_000,invite.expires_at_unix_ms as i64);
        let proof=format!("bluefin-mesh-device-v1:{}:{}:{}:{}",hex(&self.authority),hex(&self.epoch),challenge,hex(&request_hash));
        let response=serde_json::to_value(Envelope::new(Response::BeginResult {
            request_id,challenge_id:challenge.clone(),expires_at_unix_ms:expires as u64,
            passkey_options:json!({"kind":kind,"rpId":self.rp_id,"origin":self.origin,"publicKey":public["publicKey"],"browserUrl":browser_url}),
            device_proof_message:proof.clone(),
        })).map_err(storage)?;
        let mut tx=self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        self.live_tx(&mut tx).await?;
        let changed=sqlx::query("UPDATE BluefinMeshInvites SET State='claimed',ClaimedDevice=?,ClaimRequestId=?,ClaimRequestHash=? WHERE Id=? AND Epoch=? AND State='issued' AND Expires>?")
            .bind(peer).bind(request_id.to_vec()).bind(request_hash.to_vec()).bind(invite.invitation_id.to_vec())
            .bind(self.epoch.to_vec()).bind(now()).execute(&mut *tx).await.map_err(storage)?.rows_affected();
        if changed!=1{return Err(used());}
        tx.commit().await.map_err(storage)?;
        pending.insert(challenge,Pending {begin_id:request_id,begin_hash:request_hash,invite,peer:peer.to_vec(),label,expires,proof,ceremony,response:response.clone(),browser_secret_hash:digest(browser_secret.as_bytes()),browser_response:None,browser_used:false,finish_attempt:None});
        Ok(response)
    }

    async fn finish(&self,state:&AppState,peer:&[u8],request_id:[u8;32],request_hash:[u8;32],challenge:&str,response:Value,signature:&[u8])->Result<Value,ApiError>{
        // Retain the ceremony until its grant and receipt commit together. Dropping
        // this future or a failed transaction must not strand an already claimed QR.
        let mut map=self.pending.lock().await;
        let pending=map.get_mut(challenge).filter(|v|v.expires>now()&&v.peer==peer).ok_or_else(used)?;
        verify_device_signature(peer,pending.proof.as_bytes(),signature).map_err(|_|failed())?;
        if response==json!({"browser":true}) && pending.browser_response.is_none() {
            return Err(ApiError::Conflict("Complete the passkey request in your browser, then retry.".into()));
        }
        // The hash includes the original passkey response and device signature.
        // Retrying after any error may only continue this exact accepted attempt.
        match pending.finish_attempt {
            Some(previous) if previous!=(request_id,request_hash)=>return Err(failed()),
            Some(_)=>{},
            None=>pending.finish_attempt=Some((request_id,request_hash)),
        }
        let claim=self.invite_row(&pending.invite).await?;
        if claim.get::<String,_>("State")!="claimed"
            || claim.get::<Option<Vec<u8>>,_>("ClaimedDevice").as_deref()!=Some(peer)
            || claim.get::<Option<Vec<u8>>,_>("ClaimRequestId").as_deref()!=Some(pending.begin_id.as_slice())
            || claim.get::<Option<Vec<u8>>,_>("ClaimRequestHash").as_deref()!=Some(pending.begin_hash.as_slice()) {
            return Err(used());
        }
        let response=if response==json!({"browser":true}) { pending.browser_response.clone().ok_or_else(failed)? } else {response};
        let (user,registration,username,credential_update)=match &pending.ceremony {
            Ceremony::Login {user,state:ceremony}=>{
                self.account_allowed(state,*user).await?;
                let credential=serde_json::from_value(response).map_err(|_|failed())?;
                let result=self.webauthn.finish_passkey_authentication(&credential,ceremony).map_err(|_|failed())?;
                let previous:String=sqlx::query_scalar("SELECT Credential FROM BluefinPasskeys WHERE UserId=? AND CredentialId=?")
                    .bind(guid_to_db(*user)).bind(result.cred_id().as_ref()).fetch_optional(self.db.pool()).await.map_err(storage)?.ok_or_else(failed)?;
                let mut key:Passkey=serde_json::from_str(&previous).map_err(storage)?;
                if key.update_credential(&result).is_none(){return Err(failed());}
                let updated=serde_json::to_string(&key).map_err(storage)?;
                (Some(*user),None,None,Some((result.cred_id().as_ref().to_vec(),previous,updated)))
            },
            Ceremony::Signup {username,state:ceremony}=>{
                if !self.policy(state).await?.allow_signups {return Err(failed());}
                let credential=serde_json::from_value(response).map_err(|_|failed())?;
                let key=self.webauthn.finish_passkey_registration(&credential,ceremony).map_err(|_|failed())?;
                (None,Some((key.cred_id().as_ref().to_vec(),serde_json::to_string(&key).map_err(storage)?)),Some(username.clone()),None)
            },
        };
        let outcome=hex(&random()?);
        let created=now();
        let descriptor=match user {
            Some(user)=>self.descriptor(state,&outcome,user,created+SESSION_MS).await?,
            None=>json!({"kind":"signup","handle":outcome,"state":"pending","server":self.address}),
        };
        let receipt=serde_json::to_value(Envelope::new(Response::FinishResult {request_id,session:descriptor})).map_err(storage)?;
        let mut tx=self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        self.live_tx(&mut tx).await?;
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM BluefinMeshReceipts").fetch_one(&mut *tx).await.map_err(storage)?;
        if count>=MAX_RECORDS {return Err(ApiError::ServiceUnavailable("The connection receipt store is full.".into()));}
        let changed=sqlx::query("UPDATE BluefinMeshInvites SET State='consumed',OutcomeId=? WHERE Id=? AND Epoch=? AND State='claimed' AND Expires>?")
            .bind(&outcome).bind(pending.invite.invitation_id.to_vec()).bind(self.epoch.to_vec()).bind(created)
            .execute(&mut *tx).await.map_err(storage)?.rows_affected();
        if changed!=1{return Err(used());}
        if let Some((id,previous,updated))=credential_update {
            let changed=sqlx::query("UPDATE BluefinPasskeys SET Credential=? WHERE UserId=? AND CredentialId=? AND Credential=?")
                .bind(updated).bind(guid_to_db(user.ok_or_else(failed)?)).bind(id).bind(previous)
                .execute(&mut *tx).await.map_err(storage)?.rows_affected();
            if changed!=1{return Err(failed());}
        }
        if let Some(user)=user {
            self.insert_session(&mut tx,&outcome,user,peer,&pending.label,created).await?;
        } else {
            let (id,key)=registration.ok_or_else(failed)?;
            sqlx::query("INSERT INTO BluefinMeshSignups(Id,InvitationId,DevicePublicKey,DeviceLabel,Username,CredentialId,Credential,Created,State) VALUES(?,?,?,?,?,?,?,?,'pending')")
                .bind(&outcome).bind(pending.invite.invitation_id.to_vec()).bind(peer).bind(&pending.label)
                .bind(username.ok_or_else(failed)?).bind(id).bind(key).bind(created).execute(&mut *tx).await.map_err(storage)?;
        }
        sqlx::query("INSERT INTO BluefinMeshReceipts(RequestId,DevicePublicKey,RequestHash,Response) VALUES(?,?,?,?)")
            .bind(request_id.to_vec()).bind(peer).bind(request_hash.to_vec()).bind(serde_json::to_string(&receipt).map_err(storage)?)
            .execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        map.remove(challenge);
        drop(map);
        if user.is_none() {
            // The committed receipt is returned even if optional provisioning fails.
            // Its original device can retry by replaying Finish or polling status.
            let _=self.resume_automatic_signup(state,peer,&outcome).await;
        }
        Ok(receipt)
    }
    async fn resume_automatic_signup(&self,state:&AppState,peer:&[u8],handle:&str)->Result<bool,ApiError>{
        if !id_valid(handle) || peer.len()!=1952 {return Err(failed());}
        // Finish drops the pending-map lock before entering here. All callers
        // hold the mesh gate, then this policy gate, then acquire DB resources.
        // The account owner does not reacquire either gate.
        let _policy_gate=state.account_gate.lock().await;
        let policy=self.policy(state).await?;
        if !policy.allow_signups || policy.mesh_signup_approval!=super::bluefin_accounts::SignupApproval::Automatic {
            return Ok(false);
        }
        self.live().await?;
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM BluefinMeshSignups WHERE Id=? AND DevicePublicKey=? AND State='pending' AND UserId IS NULL)")
            .bind(handle).bind(peer).fetch_one(self.db.pool()).await.map_err(storage)?;
        if !pending {return Ok(false);}
        // The account owner atomically checks pending state, creates one
        // restricted account and attaches its passkey. Denials remain final.
        state.users.approve_mesh_signup(handle).await?;
        Ok(true)
    }
    async fn descriptor(&self,state:&AppState,id:&str,user:Uuid,expires:i64)->Result<Value,ApiError>{
        let account=state.users.get_user_by_id(user).await?.ok_or_else(failed)?;
        Ok(json!({"kind":"session","id":id,"userId":user.simple().to_string(),"username":account.username,
            "expiresAtUnixMs":expires,"server":self.address,"apiOrigin":self.origin,"authentication":"mldsa65-device"}))
    }
    async fn insert_session(&self,tx:&mut sqlx::Transaction<'_,sqlx::Sqlite>,id:&str,user:Uuid,peer:&[u8],label:&str,created:i64)->Result<(),ApiError>{
        sqlx::query("INSERT INTO BluefinMeshSessions(Id,UserId,DevicePublicKey,DeviceLabel,DeviceId,Created,Expires) VALUES(?,?,?,?,?,?,?)")
            .bind(id).bind(guid_to_db(user)).bind(peer).bind(label).bind(format!("bluefin-mesh:{id}"))
            .bind(created).bind(created+SESSION_MS).execute(&mut **tx).await.map_err(storage)?;
        Ok(())
    }
    async fn status(&self,state:&AppState,peer:&[u8],handle:&str)->Result<Value,ApiError>{
        self.live().await?;
        if !id_valid(handle){return Err(failed());}
        let mut row=sqlx::query("SELECT * FROM BluefinMeshSignups WHERE Id=? AND DevicePublicKey=?")
            .bind(handle).bind(peer).fetch_optional(self.db.pool()).await.map_err(storage)?.ok_or_else(failed)?;
        if row.get::<String,_>("State")=="pending" && self.resume_automatic_signup(state,peer,handle).await.unwrap_or(false) {
            row=sqlx::query("SELECT * FROM BluefinMeshSignups WHERE Id=? AND DevicePublicKey=?")
                .bind(handle).bind(peer).fetch_optional(self.db.pool()).await.map_err(storage)?.ok_or_else(failed)?;
        }
        let status:String=row.get("State");
        if status!="approved" {return Ok(json!({"version":1,"body":{"type":"status_result","handle":handle,"state":status}}));}
        let stored_user:Option<String>=row.get("UserId");
        let user=Uuid::parse_str(&stored_user.ok_or_else(failed)?).map_err(|_|failed())?;
        self.account_allowed(state,user).await?;
        let stored:Option<String>=row.get("SessionId");
        let id=if let Some(id)=stored {id} else {
            let id=hex(&random()?);
            let mut tx=self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
            self.live_tx(&mut tx).await?;
            let changed=sqlx::query("UPDATE BluefinMeshSignups SET SessionId=? WHERE Id=? AND State='approved' AND SessionId IS NULL AND DevicePublicKey=?")
                .bind(&id).bind(handle).bind(peer).execute(&mut *tx).await.map_err(storage)?.rows_affected();
            if changed!=1 {return Err(ApiError::Conflict("Retry the request status.".into()));}
            self.insert_session(&mut tx,&id,user,peer,&row.get::<String,_>("DeviceLabel"),now()).await?;
            tx.commit().await.map_err(storage)?; id
        };
        let expiry:Option<i64>=sqlx::query_scalar("SELECT Expires FROM BluefinMeshSessions WHERE Id=? AND Revoked=0 AND Expires>?")
            .bind(&id).bind(now()).fetch_optional(self.db.pool()).await.map_err(storage)?;
        let session=self.descriptor(state,&id,user,expiry.ok_or_else(failed)?).await?;
        Ok(json!({"version":1,"body":{"type":"status_result","handle":handle,"state":"approved","session":session}}))
    }
    /// The caller checks the fresh TLS exporter before dispatching the request.
    pub async fn authorization(&self,state:&AppState,peer:&[u8],session_id:&str,remote_endpoint:&str)->Result<MeshAuthorization,ApiError>{
        let _gate=self.gate.lock().await;
        self.live().await?;
        if peer.len()!=1952 || !id_valid(session_id) {return Err(failed());}
        let row=sqlx::query("SELECT UserId,DeviceId,DeviceLabel FROM BluefinMeshSessions WHERE Id=? AND DevicePublicKey=? AND Revoked=0 AND Expires>?")
            .bind(session_id).bind(peer).bind(now()).fetch_optional(self.db.pool()).await.map_err(storage)?.ok_or_else(failed)?;
        let user=Uuid::parse_str(&row.get::<String,_>("UserId")).map_err(|_|failed())?;
        self.account_allowed(state,user).await?;
        let device_id:String=row.get("DeviceId");
        let token:Option<String>=sqlx::query_scalar("SELECT AccessToken FROM Devices WHERE DeviceId=? AND lower(UserId)=lower(?) ORDER BY Id LIMIT 1")
            .bind(&device_id).bind(guid_to_db(user)).fetch_optional(self.db.pool()).await.map_err(storage)?;
        let token=match token {Some(token)=>Secret::from(token),None=>{
            state.sessions.authenticate_mesh(&AuthenticationRequest {
                user_id:Some(user),device_id:Some(device_id.clone()),device_name:Some(row.get("DeviceLabel")),
                app:Some("Bluefin".into()),app_version:Some("1".into()),remote_endpoint:Some(remote_endpoint.into()),
                ..Default::default()
            }).await?.access_token
        }};
        Ok(MeshAuthorization{token,device_id})
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Status { #[serde(rename="type")] _kind:StatusType, handle:String }
#[derive(Deserialize)]
#[serde(rename_all="snake_case")]
enum StatusType {Status}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
struct Issue {intent:InviteIntent, account_id:Option<Uuid>, #[serde(default="default_lifetime")] lifetime_seconds:u64}
fn default_lifetime()->u64{600}
fn owner(state:&AppState)->Result<&Service,ApiError>{
    state.mesh_accounts.as_deref().ok_or_else(||ApiError::ServiceUnavailable("Mesh connections are not enabled on this server.".into()))
}
async fn issue(State(state):State<AppState>,RequireAuth(auth):RequireAuth,Json(input):Json<Issue>)->Result<Json<Value>,ApiError>{
    let own=auth.user.as_ref().and_then(|u|Uuid::parse_str(&u.id).ok());
    let admin=is_administrator_identity(&state,&auth).await?;
    let account=match input.intent {
        InviteIntent::Login=>{
            let account=input.account_id.or(own).ok_or_else(failed)?;
            if Some(account)!=own&&!admin {return Err(failed());} Some(account)
        },
        InviteIntent::Signup=>{if !admin||input.account_id.is_some(){return Err(failed());}None},
    };
    Ok(Json(owner(&state)?.issue(&state,input.intent,account,input.lifetime_seconds).await?))
}
async fn requests(State(state):State<AppState>,_:RequireAdmin)->Result<Json<Value>,ApiError>{
    let service=owner(&state)?;
    let rows=sqlx::query("SELECT Id,Username,DeviceLabel,Created,State FROM BluefinMeshSignups ORDER BY Created DESC LIMIT 256")
        .fetch_all(service.db.pool()).await.map_err(storage)?;
    Ok(Json(Value::Array(rows.iter().map(|row|json!({"id":row.get::<String,_>("Id"),
        "username":row.get::<String,_>("Username"),"device":row.get::<String,_>("DeviceLabel"),
        "createdAtUnixMs":row.get::<i64,_>("Created"),"state":row.get::<String,_>("State")})).collect())))
}
async fn approve(State(state):State<AppState>,_:RequireAdmin,Path(id):Path<String>)->Result<Json<Value>,ApiError>{
    let service=owner(&state)?;let _gate=service.gate.lock().await;service.live().await?;
    if !id_valid(&id){return Err(failed());}
    state.users.approve_mesh_signup(&id).await?;
    Ok(Json(json!({"state":"approved"})))
}
async fn deny(State(state):State<AppState>,_:RequireAdmin,Path(id):Path<String>)->Result<Json<Value>,ApiError>{
    let service=owner(&state)?;let _gate=service.gate.lock().await;service.live().await?;
    if !id_valid(&id){return Err(failed());}
    let changed=sqlx::query("UPDATE BluefinMeshSignups SET State='denied' WHERE Id=? AND State='pending'")
        .bind(&id).execute(service.db.writer()).await.map_err(storage)?.rows_affected();
    if changed!=1{return Err(ApiError::Conflict("This request has already been decided.".into()));}
    Ok(Json(json!({"state":"denied"})))
}
async fn invite_status(State(state):State<AppState>,RequireAuth(auth):RequireAuth,Path(id):Path<String>)->Result<Json<Value>,ApiError>{
    if !id_valid(&id){return Err(failed());}
    let bytes:Vec<u8>=(0..64).step_by(2).map(|i|u8::from_str_radix(&id[i..i+2],16).map_err(|_|failed())).collect::<Result<_,_>>()?;
    let service=owner(&state)?;service.live().await?;
    let row=sqlx::query("SELECT State,Expires,UserId,Epoch FROM BluefinMeshInvites WHERE Id=?")
        .bind(bytes).fetch_optional(service.db.pool()).await.map_err(storage)?.ok_or_else(failed)?;
    let bound:Option<String>=row.get("UserId");
    let bound=bound.as_deref().map(Uuid::parse_str).transpose().map_err(|_|failed())?;
    let own=auth.user.as_ref().and_then(|user|Uuid::parse_str(&user.id).ok());
    if !is_administrator_identity(&state,&auth).await? && (own.is_none() || bound!=own) {return Err(failed());}
    let stored:String=row.get("State");let expires:i64=row.get("Expires");
    let status=if matches!(stored.as_str(),"claimed"|"consumed") {"used"} else if stored=="revoked" || expires<=now()
        || row.get::<Vec<u8>,_>("Epoch")!=service.epoch {"expired"} else {"issued"};
    Ok(Json(json!({"state":status,"expiresAtUnixMs":expires})))
}
pub(crate) fn routes()->Router<AppState>{
    web::routes().route("/Bluefin/Accounts/Mesh/Invite",post(issue))
        .route("/Bluefin/Accounts/Mesh/Invites/{id}",get(invite_status))
        .route("/Bluefin/Accounts/Mesh/Requests",get(requests))
        .route("/Bluefin/Accounts/Mesh/Requests/{id}/Approve",post(approve))
        .route("/Bluefin/Accounts/Mesh/Requests/{id}/Deny",post(deny))
}

#[cfg(test)]
#[path = "bluefin_mesh_accounts_tests.rs"]
mod tests;

#[path = "bluefin_mesh_web.rs"]
mod web;

#[path = "bluefin_mesh_revocation.rs"]
mod revocation;
