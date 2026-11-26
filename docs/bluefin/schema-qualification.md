# Deployed database migration continuity

The continuation source inherited an external-identities migration numbered 48 and omitted the deployed item-alias migration. The deployed SQLx history assigns item aliases to version 48 and external identities to version 51.

The source now uses those deployed numbers. Both SQL bodies are unchanged; their SHA-384 checksums match the existing migration ledger. Versions 1 through 51 were compared byte-for-byte through their recorded checksums before preparing the upgrade. The item-alias SQL was recovered alone from a retained qualified Bluefin source capture; unrelated private source was not imported.

The checksum fixture retains this actual deployed history and adds the new passkey, mesh-account, and terminal-revocation migrations at versions 52–54. Application-owned additive tables are excluded from Jellyfin-owned schema parity comparison and tested through their own schema and authority tests.

A server release must pass the migration version, checksum, fresh/adopted schema, and copied-live-database upgrade gates. The read-only live snapshot is private qualification input; it must never be committed or distributed with Bluefin. This reconciliation does not modify a live database.


## Private copied-database upgrade gate

The ignored `copied_database_upgrade` integration test requires an explicitly
admitted, read-only snapshot input. It copies that fixture into bounded build
storage and invokes the same `Database::connect_sized` and `run_migrations`
entry points used by the server. It checks all 51 original migration receipts,
the source checksums for the resulting 54 versions, every existing table's
row count and row set, SQLite integrity, and foreign-key results. It then closes,
reopens, and repeats the migration to check idempotence.

The fixture is never part of this repository or a release bundle. Its hash is
declared in the private qualification input manifest. The test prints no account
rows, library contents, tokens, or database values, and deletes its writable
copy after closing all connections. A passing isolated schema fixture test does
not replace this actual copied-database upgrade gate for deployment.
