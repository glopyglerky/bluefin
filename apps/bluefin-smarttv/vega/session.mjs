// Native-shell network ownership. Tokens are private process memory; never URLs.
const grant = '[a-f0-9]{64}';
export class Session {
  #token = ''; #epoch = 0; #lease = null; #controllers = new Set();
  constructor(base, device, transport = fetch) {
    if (!/^https:\/\/[a-z0-9.-]+(?::[0-9]+)?(?:\/[a-z0-9._~-]+)*\/?$/i.test(base) || !/^[a-z0-9_-]{16,128}$/i.test(device)) throw Error('Use HTTPS and a valid process device identifier.');
    this.base = base.replace(/\/$/, ''); this.device = device; this.transport = transport;
  }
  async request(path, method = 'GET', body, anonymous = false) {
    const controller = new AbortController(); this.#controllers.add(controller);
    const timer = setTimeout(() => controller.abort(), 15000);
    try {
      const r = await this.transport(this.base + path, {method, credentials: 'omit', redirect: 'error', cache: 'no-store', referrerPolicy: 'no-referrer', signal: controller.signal,
        headers: {'Content-Type': 'application/json', Authorization: `MediaBrowser Client="Bluefin Vega", Device="Vega", DeviceId="${this.device}", Version="0.1.0"${!anonymous && this.#token ? `, Token="${this.#token}"` : ''}`}, body: body === undefined ? undefined : JSON.stringify(body)});
      if (r.status === 401) { this.#token = ''; this.#epoch++; }
      if (!r.ok) throw Error(`Bluefin refused the request (${r.status}).`);
      if (r.status === 204) return null;
      if (Number(r.headers.get('content-length')) > 2097152) throw Error('Response limit.');
      const text = await r.text(); if (text.length > 2097152) throw Error('Response limit.');
      return JSON.parse(text);
    } finally { clearTimeout(timer); this.#controllers.delete(controller); }
  }
  async connect() {
    const info = await this.request('/System/Info/Public', 'GET', undefined, true);
    if (!info?.Id && !info?.ServerName) throw Error('No media server at this address.');
    let options;
    try { options = await this.request('/Bluefin/Accounts/Options', 'GET', undefined, true); }
    catch { options = {quickConnect: await this.request('/QuickConnect/Enabled', 'GET', undefined, true).catch(() => false) === true}; }
    return {name: info.ServerName || 'Sign in', options};
  }
  async signIn(username, password, create = false) {
    return this.finishSignIn(async () => {
      if (create) await this.request('/Bluefin/Accounts/Create', 'POST', {username, password}, true);
      return this.request('/Users/AuthenticateByName', 'POST', {Username: username, Pw: password}, true);
    });
  }
  beginDeviceSignIn(passkey) {
    return this.request(passkey ? '/Bluefin/Accounts/Passkey/Initiate' : '/QuickConnect/Initiate', 'POST', {}, true);
  }
  async finishDeviceSignIn(request, passkey) {
    return this.finishSignIn(async () => {
      if (passkey) return this.request('/Bluefin/Accounts/Passkey/Exchange', 'POST', request.proof, true);
      const status = await this.request('/QuickConnect/Connect?secret=' + encodeURIComponent(request.Secret), 'GET', undefined, true);
      return status.Authenticated
        ? this.request('/Users/AuthenticateWithQuickConnect', 'POST', {Secret: request.Secret}, true)
        : {pending:true};
    });
  }
  cancelDeviceSignIn(request) {
    return request?.proof ? this.request('/Bluefin/Accounts/Passkey/Cancel', 'POST', request.proof, true) : Promise.resolve();
  }
  async finishSignIn(request) {
    const stopped = this.stop(); this.#token = ''; const epoch = this.#epoch;
    await stopped; if (epoch !== this.#epoch) throw Error('Sign-in cancelled.');
    const result = await request();
    if (epoch !== this.#epoch) throw Error('Sign-in cancelled.');
    if (result.pending === true) return null;
    if (!/^[a-z0-9_-]{1,512}$/i.test(result.AccessToken || '') || !result.User?.Id) throw Error('Invalid session.');
    this.#token = result.AccessToken;
    try { const c = await this.request('/Bluefin/TV/v1'); return c.version === 1 && c.mediaLease === true; }
    catch (e) { this.#token = ''; throw e; }
  }
  library(kind = 'Movie,Series,Episode', term = '') {
    return this.request('/Items?Recursive=true&Limit=40&Fields=Overview,RunTimeTicks&IncludeItemTypes=' + encodeURIComponent(kind) + '&SearchTerm=' + encodeURIComponent(term.slice(0,128)));
  }
  children(id) { if (!/^[a-f0-9]{32}$/i.test(id)) throw Error('Invalid item.'); return this.request('/Items?ParentId='+id+'&Limit=40'); }
  item(id) { if (!/^[a-f0-9]{32}$/i.test(id)) throw Error('Invalid item.'); return this.request('/Items/'+id); }
  guide() { const now = encodeURIComponent(new Date().toISOString()); return this.request('/LiveTv/Programs?Limit=200&IsAiring=true&MaxStartDate='+now+'&MinEndDate='+now); }
  async loadQueue() {
    const result = await this.request('/Bluefin/Music/Queue');
    if (!Number.isSafeInteger(result.revision) || !Array.isArray(result.queue?.itemIds) || result.queue.itemIds.length > 200 || !result.queue.itemIds.every(id => typeof id === 'string' && /^[a-f0-9]{32}$/i.test(id.replace(/-/g,'')))) throw Error('Invalid queue');
    return result;
  }
  saveQueue(ids,revision) {
    if (!Number.isSafeInteger(revision) || revision < 0 || !Array.isArray(ids) || ids.length > 200 || !ids.every(id => /^[a-f0-9]{32}$/i.test(id))) throw Error('Invalid queue');
    return this.request('/Bluefin/Music/Queue','PUT',{protocolVersion:1,expectedRevision:revision,queue:{itemIds:ids,cursor:ids.length ? 0 : null,shuffle:false,repeat:'off'}});
  }
  route(path, release) {
    if (typeof path !== 'string' || !new RegExp(release ? `^/Bluefin/TV/v1/leases/${grant}$` : `^/Bluefin/TV/v1/media/${grant}/[0-9]+$`).test(path)) throw Error('Invalid grant route.');
    return this.base + path;
  }
  async acquire(id, kind, startTicks = 0, tracks = {}) {
    if (!this.#token || !/^[a-f0-9]{32}$/i.test(id) || !['audio','video'].includes(kind) || !Number.isSafeInteger(startTicks) || startTicks < 0 || startTicks > 6048000000000) throw Error('Invalid playback request.');
    const stopped = this.stop(), epoch = this.#epoch;
    await stopped; if (epoch !== this.#epoch) throw Error('Playback cancelled.');
    for (const index of [tracks.audio,tracks.subtitle]) if (index != null && (!Number.isSafeInteger(index) || index < 0 || index > 1024)) throw Error('Invalid track');
    const lease = await this.request('/Bluefin/TV/v1/leases', 'POST', {version:1,itemId:id,playback:{kind,maxBitrate:8000000,startTicks,audioStreamIndex:tracks.audio ?? null,subtitleStreamIndex:tracks.subtitle ?? null}});
    // Retain a valid release route even when another response field is invalid.
    this.route(lease.releaseUrl, true);
    try {
      const url = this.route(lease.mediaUrl, false);
      if (epoch !== this.#epoch || lease.version !== 1 || typeof lease.playSessionId !== 'string' || !lease.playSessionId) throw Error('Playback cancelled or invalid.');
      this.#lease = {...lease, itemId:id}; return url;
    } catch (e) { await this.request(lease.releaseUrl, 'DELETE').catch(() => {}); throw e; }
  }
  async report(position, paused, stopped = false) {
    const l = this.#lease; if (!l || !Number.isFinite(position)) return;
    await this.request('/Sessions/Playing' + (stopped ? '/Stopped' : '/Progress'), 'POST', {ItemId:l.itemId,PlaySessionId:l.playSessionId,PositionTicks:Math.round(Math.max(0,position)*10000000),IsPaused:!!paused,PlayMethod:'Transcode'}).catch(() => {});
  }
  async stop() {
    ++this.#epoch; const lease = this.#lease; this.#lease = null;
    if (lease) await this.request(lease.releaseUrl, 'DELETE').catch(() => {});
  }
  async suspend() { ++this.#epoch; for (const c of this.#controllers) c.abort(); await this.stop(); }
  async logout() { await this.suspend(); try { await this.request('/Sessions/Logout','POST'); } finally { this.#token = ''; } }
}
