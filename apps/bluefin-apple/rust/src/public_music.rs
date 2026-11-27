//! Music search, descriptive details, artwork and related works from public catalogs.
use bluefin_catalog::{
    Attribution, CatalogId, CatalogIdentity, Coverage, CoverageScope, MediaKind,
    SourceObservation, SourceStatus, musicbrainz::musicbrainz_entity,
};
use serde_json::{json, Value};
use super::{err, Result};

fn observation(status:SourceStatus, observed_at:u64) -> SourceObservation {
    SourceObservation {
        source:"musicbrainz".into(), status, observed_at, scope:CoverageScope::LinkedAccess,
        coverage:Coverage::Partial, catalog_url:Some("https://musicbrainz.org/".into()),
        attributions:vec![Attribution{name:"MusicBrainz".into(),url:"https://musicbrainz.org/".into()}],
    }
}
fn kind(input:&Value) -> Result<MediaKind> {
    match input["kind"].as_str() {
        Some("artist") => Ok(MediaKind::Artist),
        Some("releaseGroup") => Ok(MediaKind::ReleaseGroup),
        Some("recording") => Ok(MediaKind::Recording),
        _ => Err(err("Invalid public music catalog")),
    }
}
fn identity(kind:MediaKind,id:&str) -> Option<CatalogIdentity> {
    let identity=CatalogIdentity{kind,ids:vec![CatalogId{source:"musicbrainz".into(),id:id.into()}]};
    musicbrainz_entity(&identity)?;
    Some(identity)
}
fn clean(value:&str,limit:usize) -> String {
    value.chars().filter(|ch|!ch.is_control()).take(limit).collect::<String>().trim().to_owned()
}
fn artist_credit(body:&Value) -> String {
    body["artist-credit"].as_array().into_iter().flatten().filter_map(|credit|{
        let name=credit["name"].as_str().or_else(||credit["artist"]["name"].as_str())?;
        Some(format!("{}{}",clean(name,256),credit["joinphrase"].as_str().unwrap_or("")))
    }).collect::<String>()
}
fn artwork(identity:&CatalogIdentity,body:&Value) -> Option<String> {
    let (entity,id)=musicbrainz_entity(identity)?;
    if matches!(identity.kind,MediaKind::Release|MediaKind::ReleaseGroup) {
        return Some(format!("https://coverartarchive.org/{entity}/{id}/front-250"));
    }
    if identity.kind==MediaKind::Recording {
        let release=body["releases"].as_array()?.iter().find_map(|r|{
            let id=r["id"].as_str()?;
            identity_for_release(id).map(|_|id)
        })?;
        return Some(format!("https://coverartarchive.org/release/{release}/front-250"));
    }
    None
}
fn identity_for_release(id:&str)->Option<CatalogIdentity>{identity(MediaKind::Release,id)}
fn item(kind:MediaKind,body:&Value) -> Option<Value> {
    let identity=identity(kind,body["id"].as_str()?)?;
    let title=clean(body["title"].as_str().or_else(||body["name"].as_str())?,256);
    if title.is_empty(){return None}
    let (entity,id)=musicbrainz_entity(&identity)?;
    let mut parts=Vec::new();
    let credit=artist_credit(body);
    if !credit.is_empty(){parts.push(credit);}
    for key in if kind==MediaKind::Artist {["type","country","disambiguation"]}
        else {["primary-type","first-release-date","disambiguation"]} {
        if let Some(value)=body[key].as_str().filter(|s|!s.is_empty()) {parts.push(clean(value,256));}
    }
    Some(json!({"identity":identity,"title":title,"subtitle":parts.join(" · "),
        "artwork":artwork(&identity,body),"sourceUrl":format!("https://musicbrainz.org/{entity}/{id}")}))
}
pub(super) fn search(input:&Value)->Result<Value> {
    let kind=kind(input)?;
    let term=super::text(input,"query")?.trim();
    let offset=input["offset"].as_u64().unwrap_or(0);
    let document=super::mesh_connection::runtime()?.block_on(
        super::public_access::music_client()?.search(kind,term,offset));
    let field=match kind {MediaKind::Artist=>"artists",MediaKind::ReleaseGroup=>"release-groups",_=>"recordings"};
    let empty=Value::Null;
    let body=document.body.as_ref().unwrap_or(&empty);
    let results:Vec<_>=body[field].as_array().into_iter().flatten().filter_map(|row|item(kind,row)).collect();
    Ok(json!({"results":results,"total":body["count"].as_u64().unwrap_or(0),"offset":offset,
        "sources":[observation(document.status,document.observed_at)]}))
}
pub(super) fn detail(input:&Value)->Result<Value> {
    let request=super::public_access::request(input)?;
    let client=super::public_access::music_client()?;
    let runtime=super::mesh_connection::runtime()?;
    let document=runtime.block_on(client.metadata(&request.identity));
    let Some(body)=document.body else {
        return Ok(json!({"item":Value::Null,"description":Value::Null,"facts":[],"related":[],
            "sources":[observation(document.status,document.observed_at)]}));
    };
    let Some(mut summary)=item(request.identity.kind,&body) else {
        return Err(err("Music identity does not match its catalog detail"));
    };
    let mut facts=Vec::new();
    let mut add=|label:&str,value:String|{if !value.is_empty(){facts.push(json!({"label":label,"value":value}));}};
    for (key,label) in [("type","Type"),("country","Country"),("first-release-date","First released"),
        ("date","Released"),("status","Release status"),("disambiguation","Edition or artist note")] {
        if let Some(value)=body[key].as_str(){add(label,clean(value,1000));}
    }
    if let Some(begin)=body["life-span"]["begin"].as_str(){
        let end=body["life-span"]["end"].as_str().unwrap_or("present");
        add("Active",format!("{begin} – {end}"));
    }
    if let Some(length)=body["length"].as_u64(){
        add("Duration",format!("{}:{:02}",length/60_000,(length/1000)%60));
    }
    let genres=body["genres"].as_array().into_iter().flatten().filter_map(|g|g["name"].as_str())
        .take(16).collect::<Vec<_>>().join(", ");
    add("Genres",genres);
    let isrcs=body["isrcs"].as_array().into_iter().flatten().filter_map(Value::as_str)
        .take(20).collect::<Vec<_>>().join(", ");
    add("ISRC",isrcs);
    let mut description=body["disambiguation"].as_str().map(|s|clean(s,2000));
    if request.identity.kind==MediaKind::Artist {
        if let Some(id)=request.identity.id("musicbrainz"){
            if let Some(enrichment)=super::public_catalog::artist_artwork(id,&body){
                if enrichment["artwork"].is_string(){summary["artwork"]=enrichment["artwork"].clone();}
                summary["artworkCredit"]=enrichment["artworkCredit"].clone();
                description=enrichment["description"].as_str().map(|s|clean(s,2000)).or(description);
            }
        }
    }
    let mut related=Vec::new();
    for credit in body["artist-credit"].as_array().into_iter().flatten(){
        if let Some(value)=item(MediaKind::Artist,&credit["artist"]){related.push(value);}
    }
    for group in body["release-groups"].as_array().into_iter().flatten(){
        if let Some(value)=item(MediaKind::ReleaseGroup,group){related.push(value);}
    }
    if let Some(value)=item(MediaKind::ReleaseGroup,&body["release-group"]){related.push(value);}
    for release in body["releases"].as_array().into_iter().flatten(){
        if let Some(value)=item(MediaKind::Release,release){related.push(value);}
    }
    let mut tracks=Vec::new();
    let mut track_edition=None;
    let mut sources=vec![observation(document.status,document.observed_at)];
    let edition=if request.identity.kind==MediaKind::Release {Some(body.clone())}
        else if request.identity.kind==MediaKind::ReleaseGroup {
            body["releases"].as_array().into_iter().flatten()
                .find_map(|release|identity(MediaKind::Release,release["id"].as_str()?))
                .and_then(|identity|{
                    let result=runtime.block_on(client.metadata(&identity));
                    sources.push(observation(result.status,result.observed_at));
                    result.body
                })
        } else {None};
    if let Some(edition)=edition {
        track_edition=item(MediaKind::Release,&edition);
        for medium in edition["media"].as_array().into_iter().flatten(){
            for track in medium["tracks"].as_array().into_iter().flatten(){
                if let Some(mut value)=item(MediaKind::Recording,&track["recording"]){
                    value["number"]=json!(track["number"].as_str().unwrap_or(""));
                    tracks.push(value);
                }
            }
        }
    }
    let mut seen=std::collections::HashSet::new();
    related.retain(|row|seen.insert(row["sourceUrl"].as_str().unwrap_or("").to_owned()));
    related.truncate(100);tracks.truncate(300);
    Ok(json!({"item":summary,"description":description,"facts":facts,"related":related,
        "tracks":tracks,"trackEdition":track_edition,"sources":sources}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn music_cards_preserve_entity_type_and_exact_artwork_catalog(){
        let id="a74b1b7f-71a5-4011-9441-d0b5e4122711";
        let card=item(MediaKind::ReleaseGroup,&json!({"id":id,"title":"Example",
            "artist-credit":[{"artist":{"name":"Artist"}}]})).unwrap();
        assert_eq!(card["identity"]["kind"],"releaseGroup");
        assert_eq!(card["identity"]["ids"][0]["id"],id);
        assert_eq!(card["subtitle"],"Artist");
        assert!(card["artwork"].as_str().unwrap().contains("/release-group/"));
        assert!(item(MediaKind::Recording,&json!({"id":"../../bad","title":"Bad"})).is_none());
    }
}
