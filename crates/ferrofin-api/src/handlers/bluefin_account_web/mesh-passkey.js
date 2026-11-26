"use strict";
const status = document.querySelector("#status");
const proceed = document.querySelector("#continue");
const back = document.querySelector("#return");
const [challenge, secret, extra] = location.hash.slice(1).split(".");
history.replaceState(null, "", location.pathname + location.search);
let prepared;
function decode(value) {
    return Uint8Array.from(atob(value.replace(/-/g,"+").replace(/_/g,"/")), c=>c.charCodeAt(0));
}
function encode(value) {
    return btoa(String.fromCharCode(...new Uint8Array(value))).replace(/\+/g,"-").replace(/\//g,"_").replace(/=+$/,"");
}
function prepare(options, enroll) {
    const parser = enroll ? PublicKeyCredential.parseCreationOptionsFromJSON : PublicKeyCredential.parseRequestOptionsFromJSON;
    if (parser) return parser.call(PublicKeyCredential, options);
    options.challenge = decode(options.challenge);
    if (enroll) options.user.id = decode(options.user.id);
    for (const credential of enroll ? options.excludeCredentials ?? [] : options.allowCredentials ?? []) credential.id = decode(credential.id);
    return options;
}
function serialize(credential, enroll) {
    if (credential.toJSON) return credential.toJSON();
    const response = credential.response;
    const clientDataJSON = encode(response.clientDataJSON);
    const payload = enroll
        ? {clientDataJSON,attestationObject:encode(response.attestationObject),transports:response.getTransports?.()}
        : {clientDataJSON,authenticatorData:encode(response.authenticatorData),signature:encode(response.signature),
            userHandle:response.userHandle ? encode(response.userHandle) : null};
    return {id:credential.id,rawId:encode(credential.rawId),type:credential.type,
        authenticatorAttachment:credential.authenticatorAttachment,response:payload,
        clientExtensionResults:credential.getClientExtensionResults()};
}
async function post(name, extra = {}) {
    const response = await fetch("./Passkey/" + name, {
        method:"POST",headers:{"Content-Type":"application/json"},
        body:JSON.stringify({challenge,secret,...extra}),credentials:"omit",cache:"no-store",redirect:"error",
    });
    if (!response.ok) throw new Error("This passkey request is unavailable. Return to Bluefin and start again.");
    return response.status === 204 ? undefined : response.json();
}
proceed.addEventListener("click", async () => {
    if (!prepared) return;
    proceed.disabled = true;
    try {
        const credential = prepared.enroll
            ? await navigator.credentials.create(prepared.options)
            : await navigator.credentials.get(prepared.options);
        if (!credential) throw new Error("No passkey was selected.");
        await post("Verify", {credential:serialize(credential,prepared.enroll)});
        prepared = undefined; proceed.hidden = true;
        // No assertion, invitation or bearer is placed in the callback.
        back.href = "bluefin://mesh-passkey/" + challenge;
        back.hidden = false;
        status.textContent = "Passkey verified. Return to Bluefin to finish connecting this device.";
        location.href = back.href;
    } catch (error) {
        status.textContent = error.name === "NotAllowedError" ? "Passkey canceled. You can try again." : error.message;
        proceed.disabled = false;
    }
});
async function initialize() {
    if (location.protocol !== "https:" || !/^[a-f0-9]{64}$/.test(challenge ?? "")
        || !/^[a-f0-9]{64}$/.test(secret ?? "") || extra !== undefined || !window.PublicKeyCredential) {
        status.textContent = "Open the passkey request from Bluefin in a supported browser.";
        return;
    }
    try {
        const value = await post("Options");
        const enroll = value.kind === "registration";
        prepared = {enroll,options:{publicKey:prepare(value.publicKey,enroll)}};
        proceed.textContent = enroll ? "Create passkey" : "Continue with passkey";
        proceed.hidden = false; status.textContent = ""; proceed.focus();
    } catch (error) { status.textContent = error.message; }
}
void initialize();
