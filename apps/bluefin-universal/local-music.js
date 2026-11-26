// Ephemeral ownership of user-selected File objects. This is a platform adapter:
// no account, peer cache, content hash, persistence, upload, or discovery grant.
// The opaque IDs below identify selections in this page, never equal content.
((global) => {
  'use strict';
  if (global.BluefinLocalMusic) return;

  const limits = Object.freeze({
    maxFileBytes: 2 * 1024 * 1024 * 1024,
    maxDeclaredBytes: 16 * 1024 * 1024 * 1024,
    maxItems: 512,
    maxBatchItems: 20,
    maxQueueItems: 512,
    maxTitleBytes: 512,
    prefixBytes: 64,
    maxActiveSources: 1,
    maxMetadataOperations: 1
  });
  const formats = Object.freeze({
    mp3: 'audio/mpeg', aac: 'audio/aac', m4a: 'audio/mp4',
    flac: 'audio/flac', wav: 'audio/wav', aiff: 'audio/aiff'
  });
  let owner = null;
  let pageRetired = false;
  let metadataReading = false;

  function failure(message) { return new Error(message); }
  function localId(id) { return typeof id === 'string' && /^local-file-[a-f0-9]{64}$/.test(id); }
  function random() {
    const bytes = new Uint8Array(32);
    global.crypto.getRandomValues(bytes);
    return Array.from(bytes, value => value.toString(16).padStart(2, '0')).join('');
  }
  function title(name) {
    let result = '', size = 0;
    const encode = new TextEncoder();
    for (const scalar of name.replace(/\.[^.]*$/, '')) {
      const next = /[\u0000-\u001f\u007f-\u009f]/u.test(scalar) ? ' ' : scalar;
      const bytes = encode.encode(next).byteLength;
      if (size + bytes > limits.maxTitleBytes) break;
      result += next; size += bytes;
    }
    return result.trim() || 'Imported audio';
  }
  function format(prefix, name) {
    const extension = name.split('.').pop().toLowerCase();
    const starts = text => prefix.length >= text.length
      && Array.from(text).every((value, index) => prefix[index] === value.charCodeAt(0));
    const at = (offset, text) => prefix.length >= offset + text.length
      && Array.from(text).every((value, index) => prefix[offset + index] === value.charCodeAt(0));
    const id3 = starts('ID3');
    const mpeg = prefix.length >= 2 && prefix[0] === 255 && (prefix[1] & 224) === 224
      && (prefix[1] & 6) !== 0 && (prefix[1] & 24) !== 8;
    const adts = prefix.length >= 2 && prefix[0] === 255 && (prefix[1] & 246) === 240;
    if (extension === 'mp3' && (id3 || mpeg)) return 'mp3';
    if (extension === 'aac' && (id3 || adts)) return 'aac';
    if (extension === 'm4a' && at(4, 'ftyp')) return 'm4a';
    if (extension === 'flac' && starts('fLaC')) return 'flac';
    if (extension === 'wav' && starts('RIFF') && at(8, 'WAVE')) return 'wav';
    if (['aif', 'aiff'].includes(extension) && starts('FORM') && (at(8, 'AIFF') || at(8, 'AIFC'))) return 'aiff';
    throw failure('Choose an MP3, AAC, M4A, FLAC, WAV, or AIFF audio file. Playlists and service links cannot be imported.');
  }
  function frozenQueue(queue) {
    return Object.freeze({items: Object.freeze(queue.items.slice()), currentIndex: queue.currentIndex,
      repeatMode: queue.repeatMode, shuffled: queue.shuffled});
  }

  function create(options = {}) {
    if (pageRetired) throw failure('This page has closed. Reload Bluefin to choose local music again.');
    if (owner) throw failure('This page already owns a local music library.');
    if (typeof global.File !== 'function' || typeof global.Blob !== 'function'
      || typeof global.crypto?.getRandomValues !== 'function'
      || typeof global.URL?.createObjectURL !== 'function'
      || typeof global.URL?.revokeObjectURL !== 'function'
      || typeof global.AbortController !== 'function') {
      throw failure('This browser cannot open local audio files. Use an installed Bluefin app.');
    }
    const sizeOf = Object.getOwnPropertyDescriptor(global.Blob.prototype, 'size')?.get;
    const nameOf = Object.getOwnPropertyDescriptor(global.File.prototype, 'name')?.get;
    const slice = global.Blob.prototype.slice;
    const read = global.Blob.prototype.arrayBuffer;
    if (!sizeOf || !nameOf || !slice || !read) throw failure('The browser file interface is unavailable.');

    const catalogId = random(), files = new Map(), records = new Map(), listeners = new Set();
    let totalBytes = 0, revision = 0, generation = 0, retired = false, busy = false, active = null;
    let queue = {items: [], currentIndex: null, repeatMode: 'off', shuffled: false};
    let cleanupProblem = '';

    function snapshot() {
      return Object.freeze({
        version: 1, identity: 'opaque_file_selection', catalogId, revision,
        temporary: true, retired, busy, declaredBytes: totalBytes, cleanupProblem,
        items: Object.freeze(Array.from(records.values()).map(record => Object.freeze({...record}))),
        queue: frozenQueue(queue)
      });
    }
    function emit() {
      const value = snapshot();
      for (const listener of listeners) { try { listener(value); } catch (_) { /* Presentation cannot roll back ownership. */ } }
    }
    function live() {
      if (retired || pageRetired) throw failure('This temporary library has closed. Choose your files again.');
    }
    function nextRevision() {
      if (revision >= Number.MAX_SAFE_INTEGER) throw failure('The temporary library must be reopened.');
      return revision + 1;
    }
    function known(id) {
      live();
      if (!localId(id) || !files.has(id) || !records.has(id)) throw failure('Choose this file again on this device. A saved name or ID does not grant access.');
      return records.get(id);
    }
    function changedDuring(current) {
      live();
      if (generation !== current) throw failure('The local selection changed before it could finish. Choose the files again.');
    }
    function release(lease, reason = 'released') {
      if (!lease || lease.released) return;
      lease.released = true;
      if (active === lease) active = null;
      // Detach first so an abort callback that stops the player can reenter safely.
      try { lease.controller.abort(reason); }
      catch (_) { cleanupProblem = "The local player could not confirm cancellation."; }
      try { global.URL.revokeObjectURL(lease.url); }
      catch (_) { cleanupProblem = 'The browser could not confirm release of a local audio URL. Close this tab to finish cleanup.'; }
    }

    async function addFiles(selected) {
      live();
      if (!Array.isArray(selected) || selected.length === 0 || selected.length > limits.maxBatchItems) {
        throw failure('Choose between 1 and 20 audio files at a time.');
      }
      if (busy || metadataReading) throw failure('Wait for the current local file check to finish.');
      if (files.size + selected.length > limits.maxItems) throw failure('This temporary library holds up to 512 selected files.');
      const selectedFiles = selected.slice(), pending = [];
      let incomingBytes = 0;
      for (const file of selectedFiles) {
        if (!(file instanceof global.File)) throw failure('Select files in the browser file picker. Links, folders, and peer records are not files.');
        const bytes = sizeOf.call(file), name = nameOf.call(file);
        if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > limits.maxFileBytes) {
          throw failure('Choose a nonempty audio file of at most 2 GiB.');
        }
        if (typeof name !== 'string' || name.length === 0 || name.length > 4096) throw failure('The selected file name is invalid.');
        incomingBytes += bytes;
        if (totalBytes + incomingBytes > limits.maxDeclaredBytes) throw failure('This temporary library can reference at most 16 GiB of selected audio.');
        pending.push({file, bytes, name});
      }
      const current = generation;
      nextRevision();
      busy = true; metadataReading = true; emit();
      try {
        const ids = new Set(files.keys()), admitted = [];
        for (const input of pending) {
          changedDuring(current);
          // Only a bounded container prefix is read. The complete File is never
          // buffered or hashed merely to select it; playback reads on demand.
          let prefix;
          try {
            const bytes = await read.call(slice.call(input.file, 0, limits.prefixBytes));
            if (bytes.byteLength > limits.prefixBytes) throw failure('File prefix limit.');
            prefix = new Uint8Array(bytes);
          } catch (_) { throw failure('The selected file is unavailable. Make it available on this device and choose it again.'); }
          changedDuring(current);
          let id;
          for (let attempt = 0; attempt < 8; attempt++) {
            const candidate = 'local-file-' + random();
            if (!ids.has(candidate)) { id = candidate; ids.add(candidate); break; }
          }
          if (!id) throw failure('A local selection identity could not be created. Try again.');
          const importedAtUnixMs = Date.now();
          if (!Number.isSafeInteger(importedAtUnixMs) || importedAtUnixMs < 0) throw failure('The device clock is unavailable.');
          const kind = format(prefix, input.name);
          admitted.push({file: input.file, record: {
            id, identity: 'opaque_file_selection', source: 'user_file_import',
            title: title(input.name), format: kind, byteLength: input.bytes,
            importedAtUnixMs, durationSeconds: null
          }});
        }
        changedDuring(current);
        // The entire bounded selection is published together. Equal file names,
        // sizes, or bytes remain separate selections; no deduplication is claimed.
        const next = nextRevision();
        for (const entry of admitted) { files.set(entry.record.id, entry.file); records.set(entry.record.id, entry.record); }
        totalBytes += incomingBytes; revision = next; busy = false;
        return Object.freeze({added: Object.freeze(admitted.map(entry => entry.record.id)), catalog: snapshot()});
      } finally {
        busy = false; metadataReading = false; emit();
      }
    }

    function setQueue(value) {
      live();
      if (!value || !Array.isArray(value.items) || value.items.length > limits.maxQueueItems
        || !['off', 'all', 'one'].includes(value.repeatMode) || typeof value.shuffled !== 'boolean'
        || !(value.currentIndex === null || Number.isInteger(value.currentIndex)
          && value.currentIndex >= 0 && value.currentIndex < value.items.length)
        || Object.keys(value).some(key => !['items', 'currentIndex', 'repeatMode', 'shuffled'].includes(key))) {
        throw failure('The local music queue is invalid.');
      }
      for (const id of value.items) known(id);
      if (value.currentIndex === queue.currentIndex && value.repeatMode === queue.repeatMode
        && value.shuffled === queue.shuffled && value.items.length === queue.items.length
        && value.items.every((id, index) => id === queue.items[index])) return snapshot();
      const next = nextRevision();
      queue = {items: value.items.slice(), currentIndex: value.currentIndex,
        repeatMode: value.repeatMode, shuffled: value.shuffled};
      revision = next; emit(); return snapshot();
    }
    function select(id) {
      known(id);
      const index = queue.items.indexOf(id);
      return setQueue(index >= 0 ? {...queue, currentIndex: index}
        : {items: [id], currentIndex: 0, repeatMode: queue.repeatMode, shuffled: false});
    }
    function advance(forward = true, automatic = false) {
      live();
      if (typeof forward !== 'boolean' || typeof automatic !== 'boolean') throw failure('The local queue direction is invalid.');
      if (!queue.items.length) return null;
      const current = queue.currentIndex, count = queue.items.length;
      let next;
      if (automatic && queue.repeatMode === 'one' && current !== null) next = current;
      else if (current === null) next = forward ? 0 : count - 1;
      else if (forward && current + 1 < count) next = current + 1;
      else if (!forward && current > 0) next = current - 1;
      else if (queue.repeatMode === 'all') next = forward ? 0 : count - 1;
      else next = null;
      setQueue({...queue, currentIndex: next});
      return next === null ? null : queue.items[next];
    }
    function search(text) {
      live();
      if (typeof text !== "string" || new TextEncoder().encode(text).byteLength > 256
        || /[\u0000-\u001f\u007f-\u009f]/u.test(text)) throw failure("Keep local search under 256 bytes.");
      const query = text.trim().toLowerCase();
      return Object.freeze(Array.from(records.values()).filter(record => record.title.toLowerCase().includes(query))
        .map(record => Object.freeze({...record})));
    }
    function acquireSource(id) {
      const record = known(id);
      if (active) throw failure('Stop the previous local track before opening another.');
      const url = global.URL.createObjectURL(files.get(id));
      const lease = {id, url, controller: new global.AbortController(), released: false};
      active = lease;
      return Object.freeze({
        id, url, mimeType: formats[record.format], signal: lease.controller.signal,
        release() { release(lease); emit(); },
        metadata(durationSeconds) {
          if (retired || lease.released || active !== lease || !records.has(id)) return false;
          if (!Number.isFinite(durationSeconds) || durationSeconds <= 0 || durationSeconds > Number.MAX_SAFE_INTEGER / 10000000) return false;
          if (records.get(id).durationSeconds === durationSeconds) return true;
          const next = nextRevision();
          records.set(id, {...records.get(id), durationSeconds}); revision = next; emit(); return true;
        }
      });
    }
    function remove(id) {
      const record = known(id), next = nextRevision(), current = queue.currentIndex;
      const before = current === null ? 0 : queue.items.slice(0, current).filter(value => value === id).length;
      files.delete(id); records.delete(id); totalBytes -= record.byteLength;
      const items = queue.items.filter(value => value !== id);
      const index = current === null ? null : current - before;
      queue = {...queue, items, currentIndex: index !== null && index < items.length ? index : null};
      revision = next; generation++;
      if (active?.id === id) release(active, 'removed');
      emit(); return snapshot();
    }
    function clear() {
      live();
      generation++;
      files.clear(); records.clear(); totalBytes = 0;
      queue = {items: [], currentIndex: null, repeatMode: 'off', shuffled: false};
      if (revision < Number.MAX_SAFE_INTEGER) revision++;
      release(active, 'cleared'); emit(); return snapshot();
    }
    function dispose() {
      if (retired) return;
      retired = true; generation++;
      files.clear(); records.clear(); totalBytes = 0;
      queue = {items: [], currentIndex: null, repeatMode: 'off', shuffled: false};
      if (revision < Number.MAX_SAFE_INTEGER) revision++;
      release(active, 'page_closed'); emit(); listeners.clear();
      if (owner?.dispose === dispose) owner = null;
    }
    function subscribe(listener) {
      live();
      if (typeof listener !== 'function' || listeners.size >= 8) throw failure('The local music observer is unavailable.');
      listeners.add(listener);
      try { listener(snapshot()); } catch (_) {}
      return () => listeners.delete(listener);
    }
    const api = Object.freeze({snapshot, addFiles, setQueue, select, advance, search, acquireSource, remove, clear, dispose, subscribe});
    owner = api;
    if (typeof options.onChange === 'function') subscribe(options.onChange);
    return api;
  }

  global.addEventListener?.('pagehide', () => {
    pageRetired = true;
    owner?.dispose();
  });
  Object.defineProperty(global, 'BluefinLocalMusic', {
    value: Object.freeze({version: 1, limits, isLocalId: localId, create}),
    writable: false, configurable: false
  });
})(globalThis);
