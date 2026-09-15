use anyhow::{ensure, Context as _};

/// Keep the driver's preference when it works, but do not discard all DMA-BUF
/// candidates just because the preferred allocation cannot be exported.
pub(super) fn try_modifiers<T>(
    modifiers: &[i64],
    mut probe: impl FnMut(&[i64]) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    ensure!(
        !modifiers.is_empty(),
        "consumer offered no DMA-BUF modifiers"
    );
    let first_error = match probe(modifiers) {
        Ok(result) => return Ok(result),
        Err(err) => err,
    };
    if modifiers.len() > 1 {
        for modifier in modifiers {
            if let Ok(result) = probe(std::slice::from_ref(modifier)) {
                return Ok(result);
            }
        }
    }
    Err(first_error).context("no offered DMA-BUF modifier could be allocated and exported")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_preferred_allocation_without_reprobing() {
        let mut calls = 0;
        let result = try_modifiers(&[3, 2, 0], |_| {
            calls += 1;
            Ok(2)
        });
        assert_eq!(result.unwrap(), 2);
        assert_eq!(calls, 1);
    }

    #[test]
    fn retries_individual_modifiers_after_failed_preferred_allocation() {
        let mut calls = Vec::new();
        let result = try_modifiers(&[3, 2, 0], |offered| {
            calls.push(offered.to_vec());
            ensure!(offered == [0], "export failed");
            Ok(0)
        });
        assert_eq!(result.unwrap(), 0);
        assert_eq!(calls, [vec![3, 2, 0], vec![3], vec![2], vec![0]]);
    }

    #[test]
    fn failed_probes_are_bounded_and_empty_offers_are_not_allocated() {
        for offered in [vec![], vec![0], vec![3, 2, 0]] {
            let mut calls = 0;
            assert!(try_modifiers::<()>(&offered, |_| {
                calls += 1;
                anyhow::bail!("allocation failed")
            })
            .is_err());
            let expected = match offered.len() {
                0 => 0,
                1 => 1,
                n => n + 1,
            };
            assert_eq!(calls, expected);
        }
    }
}
