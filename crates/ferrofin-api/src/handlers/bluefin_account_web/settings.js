"use strict";

const login = document.querySelector("#login");
const settings = document.querySelector("#settings");
const status = document.querySelector("#status");
const device = globalThis.crypto?.randomUUID?.();
let token = "";

function endpoint(path) {
    const root = location.pathname.slice(0, -"/Bluefin/Accounts/Admin".length);
    return root + path;
}

async function call(route, method, body) {
    const identity = 'MediaBrowser Client="Bluefin Account Settings", Device="Browser", DeviceId="' + device + '", Version="1"';
    const authorization = token ? identity + ', Token="' + token + '"' : identity;
    const response = await fetch(endpoint(route), {
        method, headers: {"Content-Type": "application/json", Authorization: authorization},
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: "omit", cache: "no-store", redirect: "error",
    });
    if (!response.ok) throw new Error(response.status === 403 || response.status === 401
        ? "Sign in with a server administrator account." : "The server could not save or load the settings.");
    if (response.status !== 204) return response.json();
}

async function logout() {
    if (token) {
        try { await call("/Sessions/Logout", "POST"); }
        finally { token = ""; }
    }
}

login.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (location.protocol !== "https:" || !device) {
        status.textContent = "Open this page over HTTPS.";
        return;
    }
    const button = login.querySelector("button");
    button.disabled = true;
    status.textContent = "";
    try {
        const username = login.elements.namedItem("username").value.trim();
        const password = login.elements.namedItem("password");
        const credentials = {Username: username, Pw: password.value};
        login.reset();
        const result = await call("/Users/AuthenticateByName", "POST", credentials);
        token = result.AccessToken;
        const value = await call("/Bluefin/Accounts/Settings", "GET");
        settings.elements.namedItem("allowSignups").checked = value.allowSignups;
        settings.elements.namedItem("meshSignupApproval").value = value.meshSignupApproval ?? "manual";
        document.querySelector("#mesh").hidden = false;
        void refreshRequests();
        login.hidden = true;
        settings.hidden = false;
    } catch (error) {
        try { await logout(); } catch {}
        status.textContent = error.message;
    } finally {
        button.disabled = false;
    }
});

settings.addEventListener("submit", async (event) => {
    event.preventDefault();
    const save = settings.querySelector("button");
    save.disabled = true;
    try {
        await call("/Bluefin/Accounts/Settings", "PUT", {
            allowSignups: settings.elements.namedItem("allowSignups").checked,
            meshSignupApproval: settings.elements.namedItem("meshSignupApproval").value,
        });
        status.textContent = "Saved.";
    } catch (error) { status.textContent = error.message; }
    finally { save.disabled = false; }
});

document.querySelector("#logout").addEventListener("click", async () => {
    try { await logout(); status.textContent = ""; }
    catch (error) { status.textContent = error.message; }
    settings.hidden = true;
    login.hidden = false;
    document.querySelector("#mesh").hidden = true;
    clearConnection();
});

let connectionPoll;
let connectionImageURL;
function clearConnection() {
    clearTimeout(connectionPoll);
    if (connectionImageURL) URL.revokeObjectURL(connectionImageURL);
    connectionImageURL = undefined;
    document.querySelector("#connection").hidden = true;
    document.querySelector("#connection-code").value = "";
    document.querySelector("#connection-image").removeAttribute("src");
    document.querySelector("#save-image").removeAttribute("href");
}
async function createCode(intent) {
    clearConnection();
    status.textContent = "";
    try {
        const value = await call("/Bluefin/Accounts/Mesh/Invite", "POST", {intent, lifetimeSeconds: 600});
        const bytes = Uint8Array.from(atob(value.pngBase64), char => char.charCodeAt(0));
        connectionImageURL = URL.createObjectURL(new Blob([bytes], {type: "image/png"}));
        document.querySelector("#connection-image").src = connectionImageURL;
        document.querySelector("#save-image").href = connectionImageURL;
        document.querySelector("#connection-code").value = value.code;
        document.querySelector("#connection").hidden = false;
        const label = document.querySelector("#connection-status");
        label.textContent = "Ready to connect. Expires " + new Date(value.expiresAtUnixMs).toLocaleTimeString() + ".";
        const check = async () => {
            if (!token || !connectionImageURL) return;
            try {
                const current = await call("/Bluefin/Accounts/Mesh/Invites/" + value.handle, "GET");
                if (current.state !== "issued" || Date.now() >= value.expiresAtUnixMs) {
                    clearConnection();
                    status.textContent = current.state === "used" ? "This connection code has been used." : "This connection code has expired.";
                    void refreshRequests();
                    return;
                }
                connectionPoll = setTimeout(check, 2000);
            } catch {
                clearConnection();
                status.textContent = "Connection status is unavailable. Create a fresh code when the server is reachable.";
            }
        };
        connectionPoll = setTimeout(check, 2000);
    } catch (error) { status.textContent = error.message; }
}
async function refreshRequests() {
    const list = document.querySelector("#signup-requests");
    list.replaceChildren();
    try {
        const rows = await call("/Bluefin/Accounts/Mesh/Requests", "GET");
        if (!rows.length) { list.textContent = "No signup requests."; return; }
        for (const row of rows) {
            const entry = document.createElement("p");
            const label = document.createElement("span");
            label.textContent = row.username + " — " + row.device + " — " + row.state + " ";
            entry.append(label);
            if (row.state === "pending") for (const action of ["Approve", "Deny"]) {
                const button = document.createElement("button");
                button.type = "button"; button.textContent = action;
                button.addEventListener("click", async () => {
                    button.disabled = true;
                    try {
                        await call("/Bluefin/Accounts/Mesh/Requests/" + row.id + "/" + action, "POST");
                        await refreshRequests();
                    } catch (error) { status.textContent = error.message; button.disabled = false; }
                });
                entry.append(button);
            }
            list.append(entry);
        }
    } catch (error) { list.textContent = error.message; }
}
document.querySelector("#login-code").addEventListener("click", () => createCode("login"));
document.querySelector("#signup-code").addEventListener("click", () => createCode("signup"));
document.querySelector("#refresh-requests").addEventListener("click", refreshRequests);
document.querySelector("#copy-code").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText(document.querySelector("#connection-code").value); status.textContent = "Connection code copied."; }
    catch { status.textContent = "Select and copy the connection code."; }
});
