import {test} from 'node:test';
import assert from 'node:assert/strict';
import {grantResource} from './media-policy.mjs';
const root='https://fixture.invalid/Bluefin/TV/v1/media/'+'a'.repeat(64)+'/0';
test('adaptive manifests and segments must use the same exact grant',() => {
 assert.equal(grantResource(root,root.slice(0,-1)+'123'),root.slice(0,-1)+'123');
 for(const uri of [root+'?token=secret',root.replace('a'.repeat(64),'b'.repeat(64)),root.replace('fixture.invalid','foreign.invalid'),root+'/../1',root+'#x','https://user:pass@fixture.invalid/','file:///tmp/media','1']) assert.throws(() => grantResource(root,uri));
});
