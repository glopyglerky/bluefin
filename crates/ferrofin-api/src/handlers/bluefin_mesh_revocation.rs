//! Device-key authenticated revocation retains one receipt per session.
use super::*;

impl Service {
    /// The transport verifies the current TLS exporter before calling this.
    pub(crate) async fn revoke_session(
        &self, state: &AppState, peer: &[u8], session_id: &str, request_id: [u8; 32],
    ) -> Result<Value, ApiError> {
        let _gate = self.gate.lock().await;
        if peer.len() != 1952 || !id_valid(session_id) || request_id == [0; 32] {
            return Err(failed());
        }
        let response = json!({"version":1,"body":{"type":"session_revoked",
            "session_id":session_id,"request_id":request_id}});
        // Retries use their fresh connection exporter, excluded from this hash.
        let request_hash = digest(&serde_json::to_vec(&json!({
            "operation":"bluefin/revoke-session/1","session_id":session_id,
            "request_id":request_id,"device_public_key":peer,
        })).map_err(storage)?);
        let mut tx = self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        self.live_tx(&mut tx).await?;
        let session = sqlx::query(
            "SELECT UserId,DeviceId FROM BluefinMeshSessions WHERE Id=? AND DevicePublicKey=?"
        ).bind(session_id).bind(peer).fetch_optional(&mut *tx).await.map_err(storage)?
            .ok_or_else(failed)?;
        let user = Uuid::parse_str(&session.get::<String, _>("UserId")).map_err(|_| failed())?;
        let device_id: String = session.get("DeviceId");
        if let Some(receipt) = sqlx::query(
            "SELECT RequestId,DevicePublicKey,RequestHash,Response FROM BluefinMeshSessionRevocations WHERE SessionId=?"
        ).bind(session_id).fetch_optional(&mut *tx).await.map_err(storage)? {
            if receipt.get::<Vec<u8>, _>("RequestId") != request_id
                || receipt.get::<Vec<u8>, _>("DevicePublicKey") != peer
                || receipt.get::<Vec<u8>, _>("RequestHash") != request_hash
                || serde_json::from_str::<Value>(&receipt.get::<String, _>("Response"))
                    .map_err(storage)? != response {
                return Err(failed());
            }
        } else {
            // One receipt per session bounds storage for adversarial retries.
            sqlx::query(
                "INSERT INTO BluefinMeshSessionRevocations(SessionId,RequestId,DevicePublicKey,RequestHash,Response) VALUES(?,?,?,?,?)"
            ).bind(session_id).bind(request_id.to_vec()).bind(peer).bind(request_hash.to_vec())
                .bind(serde_json::to_string(&response).map_err(storage)?)
                .execute(&mut *tx).await.map_err(storage)?;
        }
        // Existing SQL triggers remove the bearer and prevent reminting.
        sqlx::query("UPDATE BluefinMeshSessions SET Revoked=1 WHERE Id=? AND DevicePublicKey=?")
            .bind(session_id).bind(peer).execute(&mut *tx).await.map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        // Cleanup retries need no duplicate bearer in the receipt.
        self.close_device_sessions(state, user, &device_id).await?;
        Ok(response)
    }

    /// Administrator removal addresses the durable grant, including a device
    /// that has not minted an HTTP bearer yet or whose cleanup needs a retry.
    /// It does not impersonate the device or manufacture a self-revoke receipt.
    pub(crate) async fn revoke_devices_by_admin(
        &self,
        state: &AppState,
        _admin: &RequireAdmin,
        device_ids: &[&str],
    ) -> Result<(), ApiError> {
        let _gate = self.gate.lock().await;
        let mut tx = self.db.writer().begin_with("BEGIN IMMEDIATE").await.map_err(storage)?;
        self.live_tx(&mut tx).await?;
        let mut targets = Vec::new();
        let mut bearers = Vec::new();
        // Admit the entire batch before changing any grant. Already revoked
        // grants remain addressable so interrupted cleanup can finish.
        for device_id in device_ids.iter().copied().collect::<std::collections::BTreeSet<_>>() {
            let valid = device_id.strip_prefix("bluefin-mesh:").is_some_and(id_valid);
            let row = if valid {
                sqlx::query("SELECT UserId FROM BluefinMeshSessions WHERE DeviceId=?")
                    .bind(device_id).fetch_optional(&mut *tx).await.map_err(storage)?
            } else {
                None
            };
            let row = row.ok_or_else(|| ApiError::BadRequest(format!("unknown device {device_id}")))?;
            let user = Uuid::parse_str(&row.get::<String, _>("UserId")).map_err(storage)?;
            targets.push((user, device_id));
            // The terminal-grant trigger removes these rows on commit. Retain
            // them only until the existing device owner invalidates its cache.
            bearers.extend(sqlx::query_as::<_, ferrofin_db::entities::security::DeviceEntity>(
                "SELECT * FROM Devices WHERE DeviceId=?"
            ).bind(device_id).fetch_all(&mut *tx).await.map_err(storage)?);
        }
        for (_, device_id) in &targets {
            sqlx::query("UPDATE BluefinMeshSessions SET Revoked=1 WHERE DeviceId=?")
                .bind(device_id).execute(&mut *tx).await.map_err(storage)?;
        }
        tx.commit().await.map_err(storage)?;
        // Security state is terminal before any fallible playback cleanup.
        // The owner gate prevents an authorization request racing a remint.
        for bearer in &bearers {
            state.devices.delete_device(bearer).await?;
        }
        for (user, device_id) in targets {
            self.close_device_sessions(state, user, device_id).await?;
        }
        Ok(())
    }

    async fn close_device_sessions(
        &self, state: &AppState, user: Uuid, device_id: &str,
    ) -> Result<(), ApiError> {
        for session in state.sessions.get_sessions(user, Some(device_id), None, None, true).await? {
            if session.user_id == user && session.device_id.as_deref() == Some(device_id) {
                if let Some(id) = session.id {
                    state.sessions.close_if_needed(&id).await?;
                }
            }
        }
        Ok(())
    }
}
