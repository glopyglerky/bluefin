-- The account owner retains at most one immutable revoke receipt per session.
-- No device secret or bearer token is copied into this table.
CREATE TABLE BluefinMeshSessionRevocations (
 SessionId TEXT PRIMARY KEY REFERENCES BluefinMeshSessions(Id) ON DELETE CASCADE,
 RequestId BLOB NOT NULL UNIQUE CHECK(length(RequestId)=32),
 DevicePublicKey BLOB NOT NULL CHECK(length(DevicePublicKey)=1952),
 RequestHash BLOB NOT NULL CHECK(length(RequestHash)=32),
 Response TEXT NOT NULL
);
CREATE TRIGGER BluefinMeshSessionRevocationImmutable
 BEFORE UPDATE ON BluefinMeshSessionRevocations
 BEGIN SELECT RAISE(ABORT,'mesh revocation receipt is immutable'); END;
CREATE TRIGGER BluefinMeshSessionRevokedIsTerminal
 BEFORE UPDATE OF Revoked ON BluefinMeshSessions
 WHEN OLD.Revoked=1 AND NEW.Revoked!=1
 BEGIN SELECT RAISE(ABORT,'mesh session revocation is terminal'); END;
