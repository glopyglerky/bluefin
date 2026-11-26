-- Retains client-visible IDs when an upstream duplicate merge removes a row.
-- Targets are checked transactionally by adoption repairs and again on reads.
CREATE TABLE "ItemIdAliases" (
    "OldItemId" TEXT NOT NULL PRIMARY KEY,
    "CanonicalItemId" TEXT NOT NULL,
    "ItemType" TEXT NOT NULL,
    CONSTRAINT "CK_ItemIdAliases_NotSelf" CHECK (lower("OldItemId") <> lower("CanonicalItemId")),
    CONSTRAINT "FK_ItemIdAliases_BaseItems_CanonicalItemId" FOREIGN KEY ("CanonicalItemId")
        REFERENCES "BaseItems" ("Id") ON DELETE CASCADE
);
CREATE INDEX "IX_ItemIdAliases_CanonicalItemId"
    ON "ItemIdAliases" ("CanonicalItemId");
