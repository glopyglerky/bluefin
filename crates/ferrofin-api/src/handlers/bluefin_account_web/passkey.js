"use strict";

const form = document.querySelector("#account");
const status = document.querySelector("#status");
const signIn = document.querySelector("#signin");
const register = document.querySelector("#register");
const ticket = location.hash.slice(1);
const password = form.elements.namedItem("password");
const proceed = document.querySelector("#continue");
let prepared = null;

function decode(value) {
    const bytes = atob(value.replace(/-/g, "+").replace(/_/g, "/"));
    return Uint8Array.from(bytes, (character) => character.charCodeAt(0));
}

function encode(value) {
    return btoa(String.fromCharCode(...new Uint8Array(value)))
        .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function prepare(options, enroll) {
    const parser = enroll
        ? PublicKeyCredential.parseCreationOptionsFromJSON
        : PublicKeyCredential.parseRequestOptionsFromJSON;
    if (parser) return parser.call(PublicKeyCredential, options);
    options.challenge = decode(options.challenge);
    if (enroll) options.user.id = decode(options.user.id);
    for (const key of enroll ? options.excludeCredentials ?? [] : options.allowCredentials ?? []) {
        key.id = decode(key.id);
    }
    return options;
}

function serialize(credential, enroll) {
    if (credential.toJSON) return credential.toJSON();
    const response = credential.response;
    const clientDataJSON = encode(response.clientDataJSON);
    const payload = enroll
        ? {clientDataJSON, attestationObject: encode(response.attestationObject), transports: response.getTransports?.()}
        : {clientDataJSON, authenticatorData: encode(response.authenticatorData), signature: encode(response.signature),
            userHandle: response.userHandle ? encode(response.userHandle) : null};
    return {id: credential.id, rawId: encode(credential.rawId), type: credential.type,
        authenticatorAttachment: credential.authenticatorAttachment, response: payload,
        clientExtensionResults: credential.getClientExtensionResults()};
}

async function post(operation, value) {
    const response = await fetch("./Passkey/" + operation, {
        method: "POST", headers: {"Content-Type": "application/json"}, body: JSON.stringify(value),
        credentials: "omit", cache: "no-store", redirect: "error",
    });
    if (!response.ok) throw new Error("Unable to verify this account. You may need to start a new request on your device.");
    return response.status === 204 ? undefined : response.json();
}

async function authorize(enroll) {
    if (!form.reportValidity()) return;
    if (enroll && !password.value) {
        password.focus();
        return;
    }
    signIn.disabled = register.disabled = true;
    try {
        const request = {ticket, username: form.elements.namedItem("username").value.trim()};
        if (enroll) request.password = password.value;
        password.value = "";
        const {publicKey} = await post("Challenge", request);
        delete request.password;
        prepared = {enroll, options: {publicKey: prepare(publicKey, enroll)}};
        form.hidden = true;
        proceed.hidden = false;
        proceed.textContent = enroll ? "Create passkey" : "Continue with passkey";
        status.textContent = "";
        proceed.focus();
    } catch (error) {
        status.textContent = error.message;
        signIn.disabled = register.disabled = false;
    }
}

proceed.addEventListener("click", async () => {
    if (!prepared) return;
    proceed.disabled = true;
    const {enroll, options} = prepared;
    try {
        // Invoke the platform authenticator directly from this button's gesture.
        const credential = enroll ? await navigator.credentials.create(options) : await navigator.credentials.get(options);
        if (!credential) throw new Error("No passkey was selected.");
        await post("Verify", {ticket, credential: serialize(credential, enroll)});
        proceed.hidden = true;
        prepared = null;
        status.textContent = "Approved. Return to Bluefin on your device.";
    } catch (error) {
        status.textContent = error.name === "NotAllowedError"
            ? "The passkey request was canceled. You can try again." : error.message;
        proceed.disabled = false;
    }
});

if (!/^[a-f0-9]{32}$/.test(ticket) || !window.PublicKeyCredential) {
    form.hidden = true;
    status.textContent = "Open the link from Bluefin in a browser that supports passkeys.";
} else {
    document.querySelector("#code").textContent = ticket.slice(0, 6).toUpperCase();
    form.addEventListener("submit", (event) => {
        event.preventDefault();
        void authorize(false);
    });
    register.addEventListener("click", () => void authorize(true));
}
