// Every media subrequest stays inside the exact opaque admission grant.
export function grantResource(root, candidate) {
  const pattern = /^(https:\/\/[a-z0-9.-]+(?::[0-9]+)?(?:\/[a-z0-9._~-]+)*\/Bluefin\/TV\/v1\/media\/[a-f0-9]{64}\/)[0-9]+$/i;
  const match = pattern.exec(root);
  if (!match || typeof candidate !== 'string' || !candidate.startsWith(match[1]) || !/^[0-9]+$/.test(candidate.slice(match[1].length))) throw Error('Invalid media capability');
  return candidate;
}
