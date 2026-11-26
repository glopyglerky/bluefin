-- Additive account-owner extension. Users, BluefinPasskeys and Devices remain
-- the identity, passkey and bearer stores.
CREATE TABLE BluefinMeshAuthority (
 Id INTEGER PRIMARY KEY CHECK(Id=1),
 AuthorityId BLOB NOT NULL CHECK(length(AuthorityId)=32),
 LiveEpoch BLOB NOT NULL CHECK(length(LiveEpoch)=32),
 Generation INTEGER NOT NULL CHECK(Generation>0)
);
CREATE TABLE BluefinMeshInvites (
 Id BLOB PRIMARY KEY CHECK(length(Id)=32),
 Epoch BLOB NOT NULL CHECK(length(Epoch)=32),
 CapsuleHash BLOB NOT NULL CHECK(length(CapsuleHash)=32),
 Intent TEXT NOT NULL CHECK(Intent IN ('login','signup')),
 UserId TEXT REFERENCES Users(Id) ON DELETE CASCADE,
 Expires INTEGER NOT NULL,
 State TEXT NOT NULL CHECK(State IN ('issued','claimed','consumed','revoked')),
 OutcomeId TEXT, ClaimedDevice BLOB, ClaimRequestId BLOB, ClaimRequestHash BLOB,
 CHECK((Intent='login' AND UserId IS NOT NULL) OR (Intent='signup' AND UserId IS NULL))
);
CREATE INDEX BluefinMeshInviteExpiry ON BluefinMeshInvites(Epoch,State,Expires);
CREATE TABLE BluefinMeshSignups (
 Id TEXT PRIMARY KEY CHECK(length(Id)=64),
 InvitationId BLOB NOT NULL UNIQUE REFERENCES BluefinMeshInvites(Id),
 DevicePublicKey BLOB NOT NULL CHECK(length(DevicePublicKey)=1952),
 DeviceLabel TEXT NOT NULL, Username TEXT NOT NULL,
 CredentialId BLOB NOT NULL UNIQUE, Credential TEXT NOT NULL,
 Created INTEGER NOT NULL,
 State TEXT NOT NULL CHECK(State IN ('pending','approved','denied')),
 UserId TEXT REFERENCES Users(Id) ON DELETE SET NULL, SessionId TEXT
);
CREATE TABLE BluefinMeshSessions (
 Id TEXT PRIMARY KEY CHECK(length(Id)=64),
 UserId TEXT NOT NULL REFERENCES Users(Id) ON DELETE CASCADE,
 DevicePublicKey BLOB NOT NULL CHECK(length(DevicePublicKey)=1952),
 DeviceLabel TEXT NOT NULL, DeviceId TEXT NOT NULL UNIQUE,
 Created INTEGER NOT NULL, Expires INTEGER NOT NULL,
 Revoked INTEGER NOT NULL DEFAULT 0 CHECK(Revoked IN (0,1))
);
CREATE TABLE BluefinMeshReceipts (
 RequestId BLOB PRIMARY KEY CHECK(length(RequestId)=32),
 DevicePublicKey BLOB NOT NULL CHECK(length(DevicePublicKey)=1952),
 RequestHash BLOB NOT NULL CHECK(length(RequestHash)=32), Response TEXT NOT NULL
);
CREATE TRIGGER BluefinMeshSpendIsTerminal BEFORE UPDATE OF State ON BluefinMeshInvites
WHEN (OLD.State IN ('consumed','revoked') AND NEW.State!=OLD.State)
 OR (OLD.State='claimed' AND NEW.State='issued')
BEGIN SELECT RAISE(ABORT,'mesh invitation is terminal'); END;
CREATE TRIGGER BluefinMeshSignupIsTerminal BEFORE UPDATE OF State ON BluefinMeshSignups
WHEN OLD.State IN ('approved','denied') AND NEW.State!=OLD.State
BEGIN SELECT RAISE(ABORT,'mesh signup is terminal'); END;
CREATE TRIGGER BluefinMeshMintFence BEFORE INSERT ON Devices
WHEN substr(NEW.DeviceId,1,13)='bluefin-mesh:'
BEGIN
 SELECT CASE WHEN NOT EXISTS (
  SELECT 1 FROM BluefinMeshSessions WHERE DeviceId=NEW.DeviceId
   AND lower(UserId)=lower(NEW.UserId) AND Revoked=0
   AND Expires>unixepoch()*1000
 ) THEN RAISE(ABORT,'mesh device grant unavailable') END;
END;
CREATE TRIGGER BluefinMeshRevoke AFTER UPDATE OF Revoked ON BluefinMeshSessions
WHEN NEW.Revoked=1
BEGIN DELETE FROM Devices WHERE DeviceId=NEW.DeviceId; END;
CREATE TRIGGER BluefinMeshSessionDelete AFTER DELETE ON BluefinMeshSessions
BEGIN DELETE FROM Devices WHERE DeviceId=OLD.DeviceId; END;
CREATE TRIGGER BluefinMeshDeviceRevoked AFTER DELETE ON Devices
WHEN substr(OLD.DeviceId,1,13)='bluefin-mesh:'
BEGIN UPDATE BluefinMeshSessions SET Revoked=1 WHERE DeviceId=OLD.DeviceId; END;
