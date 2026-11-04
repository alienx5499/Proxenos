# Pre-registration lock

Locked at **2026-11-04T11:45:48Z** (UTC), before any scored benchmark run. The repository has no git history yet, so this file stands in for the pre-registration commit: the SHA-256 of every pre-registration file as it was at that time. The files below are not edited after this point. A fix would go into a new, versioned file, with the reason written here.

## Pre-registration files

| File | SHA-256 |
|---|---|
| `bench/prereg/answer_key.json` | `85f8c91f1f8ae45794a825afb3fa756f529ea1612db5fb535af45daa8d876426` |
| `bench/prereg/scenarios.json` | `0866d32937d2776c82c30e6053806c6457e08323d69f78969b10e63af4b5226e` |
| `bench/prereg/PREREGISTRATION.md` | `84268303b4cac9b5a6481a3c6e9da902c10aa78b39d3ba63e83591ef2add00c0` |
| `bench/prereg/answer_key.sha256` | `1dd6187e0e114411b3d3e00a4956183d0606271a506f3809fcc846e8f88619ea` |
| `bench/prereg/scenarios.sha256` | `deaf76fbc60bbf3f9a81c2c4e8afb40e0f0b8c434e750f3970a6d4ff8f0a3736` |
| `bench/prereg/example.answer_key.json` | `77314f19639d928437f0da7fe9f954b350acd07b6569bfe96e1c711ed78d2bbb` |
| `bench/prereg/example.scenarios.json` | `731864140c6697cebcca6941acda434593dcbd391e1db39da9a077d290056ce2` |

`answer_key.json` and `scenarios.json` match the hashes pinned in `answer_key.sha256` and `scenarios.sha256`; the bench refuses to run or rescore otherwise.

## Run inputs (not pre-registered, recorded for reproducibility)

| Input | SHA-256 |
|---|---|
| `docs/AGENT.md` (negotiator guide read by the bench) | `b45ff7c9f416655e4b8e18e4a454e126525b7dc90e3adacca72c2e0cac5d4c6f` |
| `proxenos-bench` binary used for the runs | `8cb33903d69cea48297cae42786c44d3d4d58241e8fe0960c016f28e12b8bae5` |
| `proxenos` notary binary used for the runs | `de91e9c7dbcfe5dfed90ba74fe1da30dab894945517b6c0a05c5b69a6314ae05` |

Those three rows describe the first run (4 Nov, 12 runs). Arms A and B were rerun on 4 and 5 Nov after the bench fixes in `RUN_NOTES.md`; arm D was not rerun. Inputs for the rerun:

| Input | SHA-256 |
|---|---|
| `docs/AGENT.md` (documents the hold wake and the six-refusal cap) | `1bb353b59a3afbe48b2da346a6c236bef1484ac878efc6f436f86d7524266ebe` |
| `proxenos-bench` binary, final arms A and B run | `6689754f2bc1a1c79e23513ef1cc437bc3b88551abd92a0c7cb75ed2e3aba7bc` |
| `proxenos` notary binary (hold wake, six refusal wakes per round) | `4c35b7896db5b514038597797b9a4dacaf881af466d29804b8545ac90adc167b` |

The pre-registration files above were not changed.

## Prices

The scenario prices are the green-coffee and rent ladders described in `PREREGISTRATION.md` (USD 7.62 to 11.63 per kg; rents USD 962 to 1,162). They are deliberately not the README demo ladder (buyer opens $20.50/kg, seller floor $21.40, buyer max $22.80, injected $24.50, 400 kg), which belongs to the roasted-coffee demo deal and the kill-test fixture. A single price pair cannot give the pre-registered mix of wide, narrow, knife-edge and no-ZOPA scenarios, and the demo's round numbers would break the leak-scan margins (bounds at least 1% from every quarter-dollar). The scenario files were not changed; they were locked as they were.
