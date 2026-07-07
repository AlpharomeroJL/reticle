// The untrusted-Range parser for the archive Worker.
//
// The `Range` header is attacker controlled, so this function is the trust
// boundary between the network and the R2 `get({ range })` call. It never throws
// and never allocates against a caller-supplied length: it returns one of three
// verdicts and lets the Worker translate them into HTTP:
//
//   { type: 'full' }                         -> serve the whole object (200)
//   { type: 'range', offset, length }        -> serve those bytes (206), already
//                                               clamped to [0, size)
//   { type: 'unsatisfiable' }                -> 416, with `Content-Range: bytes */size`
//
// RFC 7233 says a syntactically invalid Range MAY be ignored (served as 200). This
// Worker deliberately does not: a malformed range is answered 416 so a probing
// client gets a clear, bounded signal instead of a silent full-object transfer.
// Every returned `offset`/`length` pair is guaranteed to satisfy
// `0 <= offset` and `offset + length <= size`, so the R2 read can never run past
// the object and can never be asked for a negative or absurd length.

/**
 * @param {string|null|undefined} rangeHeader raw value of the `Range` request header
 * @param {number} size total size of the object in bytes
 * @returns {{type:'full'}|{type:'range',offset:number,length:number}|{type:'unsatisfiable'}}
 */
export function parseRange(rangeHeader, size) {
  if (!rangeHeader) return { type: 'full' };

  // Only the `bytes` unit is supported; anything else is refused, not ignored.
  const eq = rangeHeader.indexOf('=');
  if (eq < 0) return { type: 'unsatisfiable' };
  const unit = rangeHeader.slice(0, eq).trim();
  if (unit !== 'bytes') return { type: 'unsatisfiable' };

  const spec = rangeHeader.slice(eq + 1).trim();
  // A comma means a multi-range request; this Worker does not emit multipart bodies.
  if (spec.length === 0 || spec.includes(',')) return { type: 'unsatisfiable' };

  const dash = spec.indexOf('-');
  if (dash < 0 || spec.indexOf('-', dash + 1) >= 0) return { type: 'unsatisfiable' };

  const startText = spec.slice(0, dash).trim();
  const endText = spec.slice(dash + 1).trim();

  // Suffix range: `bytes=-N` means the final N bytes.
  if (startText.length === 0) {
    const suffix = parseNonNegativeInt(endText);
    if (suffix === null || suffix === 0 || size === 0) return { type: 'unsatisfiable' };
    const length = Math.min(suffix, size);
    return { type: 'range', offset: size - length, length };
  }

  const start = parseNonNegativeInt(startText);
  if (start === null) return { type: 'unsatisfiable' };
  // A start at or past the end can never be satisfied (this also rejects size === 0).
  if (start >= size) return { type: 'unsatisfiable' };

  // Open-ended range: `bytes=start-` runs to the last byte.
  if (endText.length === 0) {
    return { type: 'range', offset: start, length: size - start };
  }

  const end = parseNonNegativeInt(endText);
  if (end === null || end < start) return { type: 'unsatisfiable' };
  const clampedEnd = Math.min(end, size - 1);
  return { type: 'range', offset: start, length: clampedEnd - start + 1 };
}

// Parses a run of ASCII digits into a safe integer. Rejects signs, decimals, hex,
// whitespace-embedded, and empty input, and rejects anything beyond the safe-integer
// range so arithmetic downstream cannot silently lose precision.
function parseNonNegativeInt(text) {
  if (!/^[0-9]+$/.test(text)) return null;
  const value = Number(text);
  if (!Number.isSafeInteger(value)) return null;
  return value;
}
