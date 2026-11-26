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
});
