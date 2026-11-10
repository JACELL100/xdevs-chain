//! Read/write helpers for the [`CommitCertificates`] table (Phase 2, research doc §6.5).
//!
//! Values are the opaque RLP bytes of `xdevs_consensus::CommitCertificate`; storage never
//! decodes them, so it does not depend on the consensus crate. The node writes the
//! certificate of block `h` in the **same** [`WriteBatch`] as the block itself (one atomic
//! commit) and may later replace it with one that carries more signatures (`Finalized`).

use crate::{
    meta_keys,
    tables::{CommitCertificates, Meta},
    DbError, DbRead, DbReadExt, Decode, WriteBatch,
};
use xdevs_primitives::Bytes;

/// Queue `cert` (RLP) as the commit certificate of block `height` in `batch`
/// (replaces any certificate already stored for that height).
pub fn put_commit_certificate<'a>(
    batch: &'a mut WriteBatch,
    height: u64,
    cert: &[u8],
) -> &'a mut WriteBatch {
    batch.put::<CommitCertificates>(&height, &Bytes::copy_from_slice(cert))
}

/// Queue the removal of the certificate of block `height` in `batch`.
pub fn delete_commit_certificate(batch: &mut WriteBatch, height: u64) -> &mut WriteBatch {
    batch.delete::<CommitCertificates>(&height)
}

/// The stored commit certificate of block `height` (RLP), if any.
pub fn commit_certificate<D: DbRead + ?Sized>(
    db: &D,
    height: u64,
) -> Result<Option<Bytes>, DbError> {
    db.get::<CommitCertificates>(&height)
}

/// The commit certificate of the head block (`Meta[LATEST_BLOCK]`) with its height.
/// `None` if there is no head yet or the head has no certificate (genesis, dev mode).
pub fn head_commit_certificate<D: DbRead + ?Sized>(
    db: &D,
) -> Result<Option<(u64, Bytes)>, DbError> {
    let Some(raw) = db.get::<Meta>(&meta_keys::LATEST_BLOCK.to_string())? else {
        return Ok(None);
    };
    let head = u64::decode(&raw)?;
    Ok(commit_certificate(db, head)?.map(|c| (head, c)))
}

/// Up to `limit` stored certificates with heights in `from..=to`, in ascending height
/// order (sync serving, debugging). Heights without a certificate are skipped.
pub fn commit_certificates_range<D: DbRead + ?Sized>(
    db: &D,
    from: u64,
    to: u64,
    limit: usize,
) -> Result<Vec<(u64, Bytes)>, DbError> {
    let mut out = Vec::new();
    if from > to || limit == 0 {
        return Ok(out);
    }
    db.walk::<CommitCertificates>(&from, |h, cert| {
        if h > to {
            return Ok(false);
        }
        out.push((h, cert));
        Ok(out.len() < limit)
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Database, Encode, MemDb, RedbDatabase};

    fn exercise(db: &dyn Database) {
        assert_eq!(head_commit_certificate(db).unwrap(), None);
        let mut b = WriteBatch::new();
        put_commit_certificate(&mut b, 1, b"c1");
        put_commit_certificate(&mut b, 2, b"c2");
        put_commit_certificate(&mut b, 4, b"c4");
        b.put::<Meta>(&meta_keys::LATEST_BLOCK.to_string(), &2u64.encode());
        db.write(b).unwrap();

        assert_eq!(commit_certificate(db, 1).unwrap().unwrap().as_ref(), b"c1");
        assert_eq!(commit_certificate(db, 3).unwrap(), None);
        let (h, c) = head_commit_certificate(db).unwrap().unwrap();
        assert_eq!((h, c.as_ref()), (2, &b"c2"[..]));

        let range = commit_certificates_range(db, 2, 10, 10).unwrap();
        assert_eq!(range.iter().map(|(h, _)| *h).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(commit_certificates_range(db, 1, 4, 2).unwrap().len(), 2);
        assert_eq!(commit_certificates_range(db, 1, 1, 10).unwrap().len(), 1);
        assert!(commit_certificates_range(db, 5, 1, 10).unwrap().is_empty());
        assert!(commit_certificates_range(db, 1, 4, 0).unwrap().is_empty());

        // A superset certificate replaces the old one; deletion removes it.
        let mut b = WriteBatch::new();
        put_commit_certificate(&mut b, 2, b"c2-more-sigs");
        delete_commit_certificate(&mut b, 4);
        db.write(b).unwrap();
        assert_eq!(
            commit_certificate(db, 2).unwrap().unwrap().as_ref(),
            b"c2-more-sigs"
        );
        assert_eq!(commit_certificate(db, 4).unwrap(), None);

        // Head without a certificate (e.g. dev mode).
        let mut b = WriteBatch::new();
        b.put::<Meta>(&meta_keys::LATEST_BLOCK.to_string(), &3u64.encode());
        db.write(b).unwrap();
        assert_eq!(head_commit_certificate(db).unwrap(), None);
    }

    #[test]
    fn helpers_on_both_backends() {
        exercise(&MemDb::new());
        let dir = tempfile::tempdir().unwrap();
        exercise(&RedbDatabase::open_dir(dir.path()).unwrap());
    }
}
