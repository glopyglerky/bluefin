'use strict';
// Run only through the admitted Forge JavaScript lane. These fixtures model the
// browser File/Blob boundary; they do not qualify Apple media decoders or Safari.
const test = require('node:test');
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const path = require('node:path');

const source = fs.readFileSync(path.join(__dirname, '../local-music.js'), 'utf8');
function fixture() {
  const blobs = new WeakMap(), names = new WeakMap(), events = new Map();
  const reads = [], created = [], revoked = [], liveUrls = new Map();
  let activeReads = 0, maximumReads = 0, random = 0, urlNumber = 0, failRevoke = false;
  class Blob {
    constructor(prefix, declaredBytes, gate) {
      const bytes = Uint8Array.from(prefix);
      blobs.set(this, {bytes, declaredBytes: declaredBytes ?? bytes.length, gate, selected: false});
    }
    get size() { return blobs.get(this).declaredBytes; }
    slice(start, end) {
      const data = blobs.get(this);
      reads.push([start, end]);
      return new Blob(data.bytes.slice(start, end), Math.max(0, Math.min(data.declaredBytes, end) - start), data.gate);
    }
    async arrayBuffer() {
      const data = blobs.get(this);
      assert.equal(data.selected, false, 'the complete selected File must not be buffered');
      activeReads++; maximumReads = Math.max(maximumReads, activeReads);
      try { if (data.gate) await data.gate; return data.bytes.slice().buffer; }
      finally { activeReads--; }
    }
  }
  class File extends Blob {
    constructor(name, prefix = Buffer.from('RIFF0000WAVEdata0123456789'), declaredBytes, gate) {
      super(prefix, declaredBytes, gate);
      names.set(this, name);
      blobs.get(this).selected = true;
    }
    get name() { return names.get(this); }
  }
  const context = vm.createContext({
    Blob, File, AbortController, TextEncoder, Uint8Array,
    crypto: {getRandomValues(bytes) { random++; for (let i = 0; i < bytes.length; i++) bytes[i] = (random >>> ((i % 4) * 8)) & 255; return bytes; }},
    URL: {
      createObjectURL(file) { assert.ok(file instanceof File); const url = 'blob:https://bluefin.test/' + (++urlNumber); created.push(file); liveUrls.set(url, file); return url; },
      revokeObjectURL(url) { revoked.push(url); if (failRevoke) throw Error('Injected browser cleanup failure'); liveUrls.delete(url); }
    },
    addEventListener(name, handler) { events.set(name, handler); },
    fetch() { throw Error('Local files must not use the network'); },
    indexedDB: undefined, WebAssembly: undefined,
    localStorage: undefined, sessionStorage: undefined,
    navigator: {}
  });
  vm.runInContext(source, context, {filename: 'local-music.js'});
  return {api: context.BluefinLocalMusic, File, reads, created, revoked, liveUrls,
    maximumReads: () => maximumReads, failRevoke: () => { failRevoke = true; },
    pagehide: () => events.get('pagehide')()};
}
function plain(value) { return JSON.parse(JSON.stringify(value)); }

test('a no-Wasm page retains File handles and only reads bounded prefixes', async () => {
  const f = fixture(), library = f.api.create();
  const a = new f.File('Concert.wav', undefined, 400 * 1024 * 1024);
  const b = new f.File('Album.flac', Buffer.from('fLaC0000000'), 900 * 1024 * 1024);
  const added = await library.addFiles([a, b]);
  assert.equal(added.catalog.busy, false);
  assert.equal(library.snapshot().declaredBytes, 1300 * 1024 * 1024);
  assert.equal(f.maximumReads(), 1);
  assert.ok(f.reads.every(([start, end]) => start === 0 && end <= 64));
  const first = library.snapshot().items[0];
  assert.equal(first.source, 'user_file_import');
  assert.equal(first.identity, 'opaque_file_selection');
  assert.equal(first.durationSeconds, null);
  assert.ok(f.api.isLocalId(first.id));
  assert.equal('hash' in first, false);
  const lease = library.acquireSource(first.id);
  assert.equal(f.created[0], a, 'the URL refers to the exact selected File, not a copy');
  assert.equal(lease.metadata(123.5), true);
  assert.equal(library.snapshot().items[0].durationSeconds, 123.5);
  lease.release();
  assert.equal(lease.signal.aborted, true);
  assert.equal(f.liveUrls.size, 0);
});

test('same names and bytes are separate selections and metadata grants no access', async () => {
  const f = fixture(), library = f.api.create(), file = new f.File('Same.wav');
  await library.addFiles([file, file]);
  const items = library.snapshot().items;
  assert.notEqual(items[0].id, items[1].id);
  assert.equal(library.snapshot().declaredBytes, file.size * 2);
  assert.throws(() => library.acquireSource('local-file-' + 'f'.repeat(64)), /does not grant access/);
  assert.throws(() => library.acquireSource('bluefin-mesh-v1:' + 'f'.repeat(64)), /does not grant access/);
  assert.throws(() => library.setQueue({items: [items[0].id], currentIndex: 0, repeatMode: 'off', shuffled: false, owner: 'another person'}), /queue is invalid/);
  const exported = plain(library.snapshot());
  library.dispose();
  const fresh = f.api.create();
  assert.throws(() => fresh.acquireSource(exported.items[0].id), /does not grant access/);
});

test('only one playback source exists and stale cleanup cannot revoke a successor', async () => {
  const f = fixture(), library = f.api.create();
  await library.addFiles([new f.File('One.wav'), new f.File('Two.wav')]);
  const [one, two] = library.snapshot().items;
  const first = library.acquireSource(one.id);
  assert.throws(() => library.acquireSource(two.id), /Stop the previous/);
  first.release();
  const second = library.acquireSource(two.id);
  first.release();
  assert.equal(f.liveUrls.has(second.url), true);
  assert.equal(f.revoked.length, 1);
  library.remove(two.id);
  assert.equal(second.signal.aborted, true);
  assert.equal(f.liveUrls.size, 0);
  second.release();
  assert.equal(f.revoked.length, 2);
});

test('page retirement clears references and refuses late metadata or new owners', async () => {
  const f = fixture(), library = f.api.create();
  await library.addFiles([new f.File('Private.wav')]);
  const lease = library.acquireSource(library.snapshot().items[0].id);
  f.pagehide();
  assert.equal(lease.signal.aborted, true);
  assert.equal(lease.metadata(50), false);
  assert.equal(library.snapshot().items.length, 0);
  assert.equal(library.snapshot().declaredBytes, 0);
  assert.equal(library.snapshot().retired, true);
  assert.equal(f.liveUrls.size, 0);
  assert.throws(() => f.api.create(), /page has closed/);
  await assert.rejects(library.addFiles([new f.File('Another.wav')]), /library has closed/);
});

test('clear during an in-flight File read cannot resurrect records or release admission early', async () => {
  const f = fixture(), library = f.api.create();
  let finish;
  const gate = new Promise(resolve => { finish = resolve; });
  const pending = library.addFiles([new f.File('Slow.wav', undefined, 1000, gate)]);
  assert.equal(library.snapshot().busy, true);
  library.clear();
  await assert.rejects(library.addFiles([new f.File('Second.wav')]), /current local file check/);
  finish();
  await assert.rejects(pending, /selection changed/);
  assert.equal(library.snapshot().items.length, 0);
  assert.equal(f.maximumReads(), 1);
  await library.addFiles([new f.File('Second.wav')]);
  assert.equal(library.snapshot().items.length, 1);
});

test('size accounting and input provenance fail before metadata work', async () => {
  const f = fixture(), library = f.api.create();
  await assert.rejects(library.addFiles(['https://example.test/song.mp3']), /file picker/);
  await assert.rejects(library.addFiles([{source: 'peer_cache', name: 'track.wav', size: 1}]), /file picker/);
  await assert.rejects(library.addFiles(Array.from({length: 21}, () => new f.File('Track.wav'))), /20 audio files/);
  await assert.rejects(library.addFiles([new f.File('Huge.wav', undefined, f.api.limits.maxFileBytes + 1)]), /at most 2 GiB/);
  assert.equal(f.reads.length, 0);
  const exact = Array.from({length: 8}, (_, i) => new f.File('Large ' + i + '.wav', undefined, 2 * 1024 * 1024 * 1024));
  await library.addFiles(exact);
  const before = f.reads.length;
  await assert.rejects(library.addFiles([new f.File('More.wav')]), /at most 16 GiB/);
  assert.equal(f.reads.length, before);
  assert.equal(library.snapshot().items.length, 8);
});

test('queue deletion, repeats and private filtering preserve local ownership', async () => {
  const f = fixture(), library = f.api.create();
  await library.addFiles(['A.wav', 'B.wav', 'C.wav'].map(name => new f.File(name)));
  const [a, b, c] = library.snapshot().items.map(item => item.id);
  library.setQueue({items: [a, b, a, c], currentIndex: 2, repeatMode: 'one', shuffled: false});
  assert.equal(library.advance(true, true), a);
  library.remove(a);
  assert.deepEqual(plain(library.snapshot().queue.items), [b, c]);
  assert.equal(library.snapshot().queue.currentIndex, 1);
  assert.equal(library.advance(true, false), null);
  library.setQueue({items: [b, c], currentIndex: 1, repeatMode: 'all', shuffled: false});
  assert.equal(library.advance(true, true), b);
  assert.equal(library.search('C')[0].id, c);
  const copy = library.snapshot();
  assert.equal(Object.isFrozen(copy.items[0]), true);
  assert.throws(() => library.setQueue({items: [a], currentIndex: 0, repeatMode: 'off', shuffled: false}), /does not grant access/);
});

test('bad container batches have no partial ownership and cleanup failure still retires', async () => {
  const f = fixture(), library = f.api.create();
  await assert.rejects(library.addFiles([new f.File('Valid.wav'), new f.File('Service.mp3', Buffer.from('#EXTM3U\nhttps://example.test/audio'))]), /Playlists and service links/);
  assert.equal(library.snapshot().items.length, 0);
  await library.addFiles([new f.File('Valid.wav')]);
  const lease = library.acquireSource(library.snapshot().items[0].id);
  f.failRevoke();
  library.dispose();
  assert.equal(lease.signal.aborted, true);
  assert.equal(library.snapshot().items.length, 0);
  assert.match(library.snapshot().cleanupProblem, /could not confirm release/);
  assert.equal(library.snapshot().retired, true);
});
