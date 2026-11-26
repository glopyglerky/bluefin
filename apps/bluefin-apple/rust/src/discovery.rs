//! Generic Bluefin discovery protocol. No provider adapter or configuration.
use super::*;
use bluefin_contract::search::SearchScope;
use bluefin_contract::discovery::{ActionForm, ActionReceipt, DiscoveryPage, SubmitAction};

pub(super) fn call(api: &Api<'_>, operation: &str) -> Result<Value> {
    match operation {
        "serverDiscoverySearch" => {
            let query = text(api.v, "query")?;
            let page = count(api.v, "page", 1, 1000)?;
            if query.trim().is_empty()
                || query.chars().count() > 160
                || query.chars().any(char::is_control)
                || page == 0
            {
                return Err(err("Invalid discovery search"));
            }
            let mut params = vec![("query", query.into()), ("page", page.to_string())];
            if let Some(scope) = api.v.get("scope").filter(|x| !x.is_null()) {
                let scope: SearchScope = serde_json::from_value(scope.clone())
                    .map_err(|_| err("Invalid search category"))?;
                if scope != SearchScope::All { params.push(("scope", scope.wire().into())); }
            }
            let page: DiscoveryPage = serde_json::from_value(api.get(
                &["Bluefin", "Discovery", "Search"], params,
            )?)
            .map_err(|_| err("Invalid discovery response"))?;
            if page.results.len() > 100
                || page.page == 0
                || page.total_pages > 1000
                || page.results.iter().any(|item| {
                    item.title.chars().count() > 256
                        || item.overview.chars().count() > 4000
                        || item.actions.len() > 4
                })
            {
                return Err(err("Discovery response exceeds bounds"));
            }
            serde_json::to_value(page).map_err(|_| err("Invalid discovery response"))
        }
        "serverDiscoveryAction" => {
            let input:SubmitAction=serde_json::from_value(json!({"actionId":api.v["actionId"],"selections":[],"authorizedUseConfirmed":false}))
                .map_err(|_|err("Invalid action"))?;
            let form: ActionForm = serde_json::from_value(api.get(
                &[
                    "Bluefin",
                    "Discovery",
                    "Actions",
                    &input.action_id.to_string(),
                ],
                vec![],
            )?)
            .map_err(|_| err("Invalid action form"))?;
            if form.id != input.action_id
                || form.choices.len() > 256
                || form.notice.chars().count() > 4096
            {
                return Err(err("Invalid action form"));
            }
            serde_json::to_value(form).map_err(|_| err("Invalid action form"))
        }
        "submitServerAction" => {
            let input:SubmitAction=serde_json::from_value(json!({"actionId":api.v["actionId"],"selections":api.v["selections"],"authorizedUseConfirmed":api.v["authorizedUseConfirmed"]}))
                .map_err(|_|err("Invalid action submission"))?;
            if !input.authorized_use_confirmed || input.selections.len() > 256 {
                return Err(err("Confirm authorized use before submitting"));
            }
            let receipt: ActionReceipt = serde_json::from_value(api.post(
                &["Bluefin", "Discovery", "Requests"],
                serde_json::to_value(input).map_err(|_| err("Invalid action"))?,
            )?)
            .map_err(|_| err("Invalid action receipt"))?;
            serde_json::to_value(receipt).map_err(|_| err("Invalid action receipt"))
        }
        _ => Err(err("Unknown discovery operation")),
    }
}
