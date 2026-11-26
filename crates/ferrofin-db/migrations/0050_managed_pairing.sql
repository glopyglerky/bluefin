-- Durable ownership and mint fence. Receipts are never evicted implicitly.
CREATE TABLE FerrofinManagedPairing (
 Id TEXT PRIMARY KEY, SecretHash TEXT NOT NULL, DeviceId TEXT NOT NULL UNIQUE,
 ClientDeviceId TEXT NOT NULL, App TEXT NOT NULL, Version TEXT NOT NULL,
 DeviceName TEXT NOT NULL, RemoteIp TEXT NOT NULL, Created INTEGER NOT NULL,
 Expires INTEGER NOT NULL, State TEXT NOT NULL CHECK(State IN ('pending','minting','authorized','exchanged','cancelled')),
 UserId TEXT
);
CREATE INDEX FerrofinManagedPairingRemote ON FerrofinManagedPairing(RemoteIp,Created);
-- SQLite serializes this check with cancellation's UPDATE and token DELETE.
CREATE TRIGGER BluefinManagedMintFence BEFORE INSERT ON Devices
WHEN substr(NEW.DeviceId,1,16) = 'bluefin-managed:'
BEGIN
 SELECT CASE WHEN NOT EXISTS (
  SELECT 1 FROM FerrofinManagedPairing WHERE DeviceId=NEW.DeviceId
  AND lower(UserId)=lower(NEW.UserId) AND State='minting' AND Expires>unixepoch()
 ) THEN RAISE(ABORT,'managed pairing mint refused') END;
END;
CREATE TRIGGER BluefinManagedMintReceipt AFTER INSERT ON Devices
WHEN substr(NEW.DeviceId,1,16) = 'bluefin-managed:'
BEGIN
 UPDATE FerrofinManagedPairing SET State='authorized' WHERE DeviceId=NEW.DeviceId;
END;
CREATE TRIGGER BluefinManagedRevoke AFTER UPDATE OF State ON FerrofinManagedPairing
WHEN NEW.State='cancelled'
BEGIN
 DELETE FROM Devices WHERE DeviceId=NEW.DeviceId;
END;
