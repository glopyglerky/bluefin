-- Additive account-owned Home signals. Existing Users rows remain the sole
-- identity authority; deleting an account also removes its personalization.
CREATE TABLE BluefinHomeState (
    UserId TEXT NOT NULL PRIMARY KEY REFERENCES Users(Id) ON DELETE CASCADE,
    Generation INTEGER NOT NULL DEFAULT 0 CHECK (Generation >= 0)
);
CREATE TABLE BluefinHomeSearches (
    UserId TEXT NOT NULL REFERENCES Users(Id) ON DELETE CASCADE,
    EventId TEXT NOT NULL,
    Generation INTEGER NOT NULL CHECK (Generation >= 0),
    Term TEXT NOT NULL CHECK (length(Term) BETWEEN 2 AND 128),
    SelectedItemId TEXT,
    OccurredAtUnixMs INTEGER NOT NULL,
    PRIMARY KEY (UserId, EventId)
);
CREATE INDEX IX_BluefinHomeSearches_UserRecent
    ON BluefinHomeSearches(UserId, OccurredAtUnixMs DESC, EventId);
CREATE INDEX IX_BluefinHomeSearches_Expiry
    ON BluefinHomeSearches(OccurredAtUnixMs);
