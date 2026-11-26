CREATE TABLE BluefinPasskeys (
    CredentialId BLOB PRIMARY KEY NOT NULL,
    UserId TEXT NOT NULL REFERENCES Users(Id) ON DELETE CASCADE,
    Credential TEXT NOT NULL
);
CREATE INDEX IX_BluefinPasskeys_UserId ON BluefinPasskeys(UserId);
