# Accounts

Enter a server address and select **Continue**. Password sign-in is available as
soon as the server responds. Bluefin checks separately for passkeys, Quick Connect,
and account creation. If that check fails, use **Retry sign-in options** to try again.

## Allow clients to create accounts

Open `/Bluefin/Accounts/Admin` under your server's HTTPS address and sign in
as an administrator. Turn on **Allow clients to create accounts**, then save.
The setting is off on a fresh server. The same setting can be provisioned by
placing the following in `bluefin-accounts.json` in the server configuration directory.

```json
{"allowSignups":true}
```

The server checks the setting when an account is created. New users choose a
password of at least 15 characters and can sign in immediately. Use the server's
user settings to grant library and Live TV access to the people who need it.

## Sign in with a passkey

Configure the server's published URL as the HTTPS address clients use. Include
the base path if the server is mounted under one. Forward account routes to
this server and preserve browser Origin headers. Passkeys are bound to that
HTTPS origin.

Choose **Sign in with a passkey** after connecting. On a TV, scan the QR code
with a phone and check the displayed confirmation code. On a computer or phone,
you can open the passkey page directly. Choose **Add a passkey** on that page
the first time, verify your current password and create the credential with
your browser's passkey prompt. Later you can choose your saved passkey.

When client signups are off, the connected Bluefin screen offers password
sign-in and passkeys if configured. Reconnect to refresh the visible choices.
The existing Quick Connect server setting still governs its compatibility API.

## Routes

- `GET /Bluefin/Accounts/Options` returns the choices for an anonymous client.
- `POST /Bluefin/Accounts/Create` accepts `username` and `password`. Use ordinary
  account authentication after successful creation.
- `GET /Bluefin/Accounts/Settings` and `PUT /Bluefin/Accounts/Settings` require an
  administrator and read or write the `allowSignups` setting.
