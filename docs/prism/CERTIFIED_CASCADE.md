# Prism: certified interval cascade (Stage C, 2026-10-27)

Status: Research Note v1.0, not peer reviewed. Statuses: **Proved** (derivation here, real arithmetic), **Proposed** (not proved or not tested), **Derived, first order** (heuristic estimate). Nothing here has been run; Stage D reproduces only the existing synthetic pilot.

Notation as in `MATH_AND_ALGORITHM.md`: `R` orthogonal rotation, `x' = Rx`, `q' = Rq`, `P` padded dimension, `s(q,x) = <q,x> = <q',x'>`.

## C.1 Valid intervals from the stored code

At `b` bits each coordinate `x'_i` falls in a cell. For an interior cell, `[l_i, u_i] = [idx*D_b - s_c, idx*D_b - s_c + D_b]` with `D_b = 2 s_c / 2^b`. The two end cells absorb clipped coordinates, so they are not bounded by `s_c`. Because `||x'|| = ||x|| = 1`, every coordinate satisfies `|x'_i| <= 1`, so the end cells are `[-1, -s_c + D_b]` and `[s_c - D_b, 1]`. These are valid and wide.

**Box bounds (Proved).** With `m_i = min(q'_i l_i, q'_i u_i)` and `M_i = max(q'_i l_i, q'_i u_i)`:

```
L_box(x) = sum_i m_i  <=  <q',x'>  <=  sum_i M_i = U_box(x)
```

Each term satisfies `m_i <= q'_i x'_i <= M_i`, and summing preserves the inequality.

**Norm bound (Proved).** Let `x_hat` be the reconstruction and `r = ||x' - x_hat||`, stored per vector. Then `|<q',x'> - <q',x_hat>| <= ||q'|| r = r` by Cauchy-Schwarz.

**Intersection (Proved).** Both hold, so `L = max(L_box, s_hat - r)` and `U = min(U_box, s_hat + r)` are valid. The intersection is never worse than either alone.

## C.2 Discard rule and exactness

Let `E` be the **eligible set**: vectors visible in the reader's snapshot, passing the filter, belonging to the caller's tenant. Let `tau` be the k-th largest `L(c)` over `c in E` (no pruning if `|E| < k`).

**Rule.** Discard `c in E` iff `U(c) < tau` (strict).

**Lemma 1 (safe under any tie policy; Proved).** A discarded `c` is not in the top-k of `E`, however ties are broken.

*Proof.* At least `k` candidates `c'` have `L(c') >= tau`; `c` is not among them, since `L(c) <= U(c) < tau`. For each, `s(c') >= L(c') >= tau > U(c) >= s(c)`. So at least `k` members of `E` score strictly above `c`. Hence `c` ranks below `k` regardless of how equal scores are ordered. QED

**Theorem 2 (exactness; Proved, real arithmetic).** If every interval is valid, ranking the survivors by their exact score, ties by ascending id, gives the same list as ranking all of `E` that way.

*Proof.* By Lemma 1 no member of the true top-k is discarded; the survivors are a superset of it. The exact top-k of a superset that contains the exact top-k, under the same total order, is the exact top-k. QED

Two preconditions make Theorem 2 about the real system, not only about real numbers:
1. The rerank uses the **same scoring function and the same f32 vectors** as the oracle, so equality is against that oracle's floating-point score.
2. Ties: `tau` is computed over `E` only. Computing it over a larger set (all tenants, all snapshots, or the unfiltered corpus) would make `tau` too high or too low and break the rule. This is a tested requirement for Stage E, not yet tested.

## C.3 Why the deterministic gate prunes almost nothing (Derived, first order)

The interval width is `W(x) = U_box - L_box = sum_i |q'_i| (u_i - l_i) = D_b * sum_i |q'_i|` for interior cells. For roughly Gaussian `q'` coordinates `E|q'_i| = sqrt(2/(pi P))`, so `sum |q'_i| ~ sqrt(2P/pi)`. With `D_b = 2C/(sqrt(P) 2^b)`:

```
W  ~  2C * sqrt(2/pi) / 2^b            e.g. C = 3, b = 4:   W ~ 0.30
typical error std  sigma_b  =  D_b / sqrt(12)  ~  0.0068      (P = 256, b = 4)
ratio  W / (2 sigma_b)  ~  sqrt(6P/pi)  ~  22                  (P = 256)
```

The worst-case interval is an `L1` quantity and the typical error an `L2` one, so the gap grows like `sqrt(P)`. A score gap between neighbours near the k-th rank is far smaller than `0.30`, so almost nothing is pruned. This matches the measured behaviour (the deterministic gate reads about 3.7 to 4.0 bits). It predicts that **no deterministic box bound can work at this dimension**, and that a probabilistic per-vector bound is the only route to both a guarantee and pruning.

## C.4 Probabilistic per-vector bound, RaBitQ style (Proposed)

RaBitQ (Theorem 3.2) gives an unbiased estimator and a bound of order `1/sqrt(D)` with failure probability `2 exp(-c0 eps0^2)`, and uses it to decide what to re-rank. Prism's analogue, if the quantization errors `e_i` behave as independent and bounded over the randomness of the rotation seed:

```
P[ |s_hat - s| > t ]  <=  2 exp( -2 t^2 / D_b^2 )          (Hoeffding, sum q'_i^2 = 1, range D_b)
t_delta = D_b * sqrt( ln(2/delta) / 2 )
```

For `P = 256`, `C = 3`, `b = 4`, `delta = 1e-3`: `D_b = 0.0234`, `t = 0.0457`, about 6.8 times the first-order `sigma_4`. That is consistent in size with the measured tails (maximum error 5.2 to 7.1 sigma over 20 million pairs), which is encouraging but not a test.

Caveats, all open:
- Independence of `e_i` is **not proved** for a three-round Hadamard rotation (correlated coordinates, section 4 of the loophole list). The probability is over the seed, per vector and per query, not simultaneously over all `n` vectors; a simultaneous guarantee costs a factor `n` in `delta`.
- The estimate is biased toward zero without a residual stage (TurboQuant's 1-bit step, or RaBitQ's `1/<o_bar,o>` correction); the bound above is for the error around the biased mean unless the bias is subtracted.
- A probabilistic gate is a different contract: exact with probability `>= 1 - delta` per query, not always exact. It must be named that way in any result.

## C.5 Floating-point, normalization, metrics, filters

**Floating point (Proposed, not implemented).** Bounds computed in `f32` can be off by rounding. A sound implementation widens each bound: `L_fp = L - eta`, `U_fp = U + eta`, with `eta` covering (a) the summation error `gamma_n sum |a_i|`, `gamma_n = n u / (1 - n u)`, `u = 2^-24`; (b) the error of the computed `q' = fl(Rq)` against `Rq`, at most `||delta_q||` added as `eps_R`; (c) the error of the stored `x'` and `r`; (d) the difference between the real score and the `f32` oracle score used for the final order. Until `eta` is derived and a differential test passes, "certified" means certified in real arithmetic only.

**Normalization.** The derivation assumes unit `x`. For a stored norm `n_x`, scale `L, U` by `n_x` (positive). The zero vector has score 0 in the oracle, so `L = U = 0`. Unnormalized queries scale `s_hat` and `t` by `||q||`.

**Metrics (each needs its own statement; none is reused blindly).**
- *Cosine on unit vectors:* equals inner product; use C.1 to C.2.
- *Inner product on stored norms:* multiply bounds by `n_x`.
- *Squared L2:* `||q - x||^2 = ||q||^2 + n_x^2 - 2<q,x>`. With exactly stored `n_x`, `D_low = ||q||^2 + n_x^2 - 2U` and `D_high = ||q||^2 + n_x^2 - 2L`. Discard `c` iff `D_low(c) > tau_D`, the k-th smallest `D_high` over `E`. Proved by the same argument with inequalities reversed. The Atlas engine ranks the negated square root, which is strictly monotone in the squared distance, so rank and ties are identical.

**Filters.** `E` is computed first (snapshot, tenant, predicate), then `tau`. If `|E| < k`, return all of `E`, ranked exactly. An empty `E` returns nothing and must not error.

## C.6 What the exact rerank costs and stores

Exact rerank needs the original `f32` vector, stored at `4d` bytes per vector, plus the per-vector residual norms (one `f32` for each of `b = 1..4`, so 16 bytes). Bytes fetched per query are `n_S * 4d` where `n_S` is the survivor count. These are included in the storage accounting on the algorithm page; the measured survivor counts and the I/O of a real tiered layout are **Unmeasured**.

## C.7 What Stage C leaves unproved

1. Floating-point margin `eta` (C.5).
2. Independence of quantization errors under the structured rotation (C.4).
3. A tight probabilistic bound for 4-plane codes; RaBitQ's bound is for 1-bit codes with an unbiased correction.
4. Multi-stage union bound: each stage re-uses the same `e_i` at different resolutions, so failure events are dependent; the union bound is valid but loose.
5. Behaviour on non-power-of-two `d` through zero padding.
