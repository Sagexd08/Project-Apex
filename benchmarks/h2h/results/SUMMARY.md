
## fashion

### Unfiltered recall@10 vs latency (median of repetitions, sequential single client)

| engine | ef | recall@10 | p50 ms | p95 ms | p99 ms | QPS seq | QPS 8 workers | reps |
|---|---|---|---|---|---|---|---|---|
| atlas | 64 | 0.9984 | 0.391 | 0.587 | 0.713 | 2461 | 12842 | 3 |
| atlas | 128 | 0.9994 | 0.586 | 0.899 | 1.111 | 1638 | 8220 | 3 |
| atlas | 256 | 0.9997 | 0.914 | 1.361 | 1.610 | 1056 | 5079 | 3 |
| atlas | 512 | 1.0000 | 1.480 | 2.175 | 2.572 | 655 | 3199 | 3 |
| atlas | 1024 | 1.0000 | 2.488 | 3.654 | 4.678 | 388 | 2064 | 3 |
| hnswlib | 64 | 0.9984 | 0.209 | 0.317 | 0.416 | 4400 | 9855 | 3 |
| hnswlib | 128 | 0.9994 | 0.323 | 0.486 | 0.589 | 2872 | 7047 | 3 |
| hnswlib | 256 | 0.9997 | 0.526 | 0.765 | 0.916 | 1799 | 5524 | 3 |
| hnswlib | 512 | 1.0000 | 0.878 | 1.276 | 1.552 | 1076 | 3581 | 3 |
| hnswlib | 1024 | 1.0000 | 1.442 | 2.063 | 2.781 | 657 | 2157 | 3 |
| qdrant | 64 | 0.9997 | 3.028 | 3.743 | 4.454 | 322 | 685 | 3 |
| qdrant | 128 | 0.9999 | 3.335 | 4.305 | 5.179 | 292 | 741 | 3 |
| qdrant | 256 | 1.0000 | 3.978 | 4.915 | 5.743 | 248 | 684 | 3 |
| qdrant | 512 | 1.0000 | 4.917 | 6.076 | 7.112 | 200 | 603 | 3 |
| qdrant | 1024 | 1.0000 | 6.168 | 7.792 | 9.077 | 159 | 530 | 3 |

### Filtered search at ef=512 (each engine's native filtered path)

| engine | selectivity | path | recall@10 | p50 ms | p95 ms | p99 ms | avg returned | leaks (200 q) |
|---|---|---|---|---|---|---|---|---|
| atlas | 10.0% | native filtered HNSW walk | 1.000 | 7.78 | 10.49 | 12.68 | 10.0 | 0 |
| atlas | 5.0% | native filtered HNSW walk | 1.000 | 13.85 | 17.82 | 21.22 | 10.0 | 0 |
| atlas | 1.0% | exact pre-filter scan over allowed ids | 1.000 | 0.58 | 0.90 | 1.32 | 10.0 | 0 |
| atlas | 0.1% | exact pre-filter scan over allowed ids | 1.000 | 0.27 | 0.38 | 0.44 | 10.0 | 0 |
| hnswlib | 10% | hnswlib native filtered walk | 1.000 | 6.44 | 8.62 | 10.95 | 10.0 | 0 |
| hnswlib | 5% | hnswlib native filtered walk | 1.000 | 11.32 | 14.88 | 19.58 | 10.0 | 0 |
| hnswlib | 1% | hnswlib native filtered walk | 1.000 | 45.77 | 64.32 | 93.27 | 10.0 | 0 |
| hnswlib | 0.1% | hnswlib native filtered walk | 1.000 | 49.62 | 61.94 | 79.31 | 10.0 | 0 |
| qdrant | 10% | qdrant native filtered search (payload index) | 1.000 | 3.31 | 4.22 | 5.45 | 10.0 | 0 |
| qdrant | 5% | qdrant native filtered search (payload index) | 1.000 | 2.87 | 4.02 | 4.75 | 10.0 | 0 |
| qdrant | 1% | qdrant native filtered search (payload index) | 1.000 | 1.84 | 2.90 | 4.10 | 10.0 | 0 |
| qdrant | 0.1% | qdrant native filtered search (payload index) | 1.000 | 1.55 | 2.32 | 3.02 | 10.0 | 0 |

### Build, memory, disk

| engine | build s | ingest vec/s | peak RSS after build MB | disk MB | notes |
|---|---|---|---|---|---|
| atlas | 41.6 | 1442 | 637 | 204.7 | embedded library via AnnIndex trait, no WAL, no API |
| hnswlib | 28.1 | 2135 | 445 | 204.7 |  |
| qdrant | 26.0 | 2305 | nan | nan | vmhwm_qdrant_process: VmHWM:	 1054396 kB VmRSS:	  550764 kB  storage_bytes: 431480657 version: qdrant 1.17.1 |

## glove

### Unfiltered recall@10 vs latency (median of repetitions, sequential single client)

| engine | ef | recall@10 | p50 ms | p95 ms | p99 ms | QPS seq | QPS 8 workers | reps |
|---|---|---|---|---|---|---|---|---|
| atlas | 64 | 0.8325 | 0.688 | 1.120 | 1.540 | 1643 | 7293 | 2 |
| atlas | 128 | 0.8893 | 1.161 | 1.930 | 2.774 | 845 | 4952 | 2 |
| atlas | 256 | 0.9336 | 1.876 | 2.927 | 3.910 | 534 | 2622 | 2 |
| atlas | 512 | 0.9659 | 3.508 | 5.520 | 7.178 | 287 | 1353 | 2 |
| atlas | 1024 | 0.9852 | 6.631 | 10.047 | 13.362 | 152 | 719 | 2 |
| hnswlib | 64 | 0.8335 | 0.269 | 0.437 | 0.572 | 3436 | 9211 | 2 |
| hnswlib | 128 | 0.8905 | 0.476 | 0.746 | 0.951 | 1998 | 6222 | 2 |
| hnswlib | 256 | 0.9342 | 0.887 | 1.364 | 1.741 | 1092 | 3845 | 2 |
| hnswlib | 512 | 0.9657 | 1.640 | 2.481 | 3.142 | 599 | 1973 | 2 |
| hnswlib | 1024 | 0.9851 | 2.983 | 4.516 | 5.687 | 331 | 1131 | 2 |
| qdrant | 64 | 0.8950 | 2.435 | 3.451 | 4.165 | 389 | 1086 | 2 |
| qdrant | 128 | 0.9421 | 3.088 | 4.287 | 5.268 | 306 | 940 | 2 |
| qdrant | 256 | 0.9737 | 4.442 | 6.072 | 7.124 | 215 | 668 | 2 |
| qdrant | 512 | 0.9908 | 6.649 | 9.292 | 11.129 | 145 | 402 | 2 |
| qdrant | 1024 | 0.9977 | 10.945 | 15.715 | 18.965 | 89 | 224 | 2 |

### Filtered search at ef=512 (each engine's native filtered path)

| engine | selectivity | path | recall@10 | p50 ms | p95 ms | p99 ms | avg returned | leaks (200 q) |
|---|---|---|---|---|---|---|---|---|
| atlas | 10.0% | native filtered HNSW walk | 0.997 | 30.23 | 43.10 | 51.43 | 10.0 | 0 |
| atlas | 5.0% | native filtered HNSW walk | 0.999 | 53.63 | 76.83 | 94.64 | 10.0 | 0 |
| atlas | 1.0% | exact pre-filter scan over allowed ids | 1.000 | 8.10 | 10.84 | 12.30 | 10.0 | 0 |
| atlas | 0.1% | exact pre-filter scan over allowed ids | 1.000 | 4.87 | 7.34 | 7.98 | 10.0 | 0 |
| hnswlib | 10% | hnswlib native filtered walk | 0.997 | 17.49 | 25.09 | 29.77 | 10.0 | 0 |
| hnswlib | 5% | hnswlib native filtered walk | 0.999 | 33.41 | 46.75 | 64.50 | 10.0 | 0 |
| hnswlib | 1% | hnswlib native filtered walk | 1.000 | 131.78 | 166.15 | 205.42 | 10.0 | 0 |
| hnswlib | 0.1% | hnswlib native filtered walk | 1.000 | 879.86 | 1071.64 | 1190.17 | 10.0 | 0 |
| qdrant | 10% | qdrant native filtered search (payload index) | 0.999 | 8.14 | 11.62 | 14.91 | 10.0 | 0 |
| qdrant | 5% | qdrant native filtered search (payload index) | 1.000 | 6.18 | 7.84 | 9.50 | 10.0 | 0 |
| qdrant | 1% | qdrant native filtered search (payload index) | 1.000 | 2.42 | 3.46 | 4.13 | 10.0 | 0 |
| qdrant | 0.1% | qdrant native filtered search (payload index) | 1.000 | 1.19 | 1.58 | 1.97 | 10.0 | 0 |

### Build, memory, disk

| engine | build s | ingest vec/s | peak RSS after build MB | disk MB | notes |
|---|---|---|---|---|---|
| atlas | 1753.0 | 675 | 1934 | 800.4 | embedded library via AnnIndex trait, no WAL, no API |
| hnswlib | 248.6 | 4760 | 1397 | 800.4 |  |
| qdrant | 501.1 | 2362 | nan | nan | vmhwm_qdrant_process: VmHWM:	 3196472 kB VmRSS:	 1497840 kB  storage_bytes: 1243970249 version: qdrant 1.17.1 |

## nytimes

### Unfiltered recall@10 vs latency (median of repetitions, sequential single client)

| engine | ef | recall@10 | p50 ms | p95 ms | p99 ms | QPS seq | QPS 8 workers | reps |
|---|---|---|---|---|---|---|---|---|
| atlas | 64 | 0.8862 | 0.923 | 1.426 | 1.842 | 1068 | 4977 | 3 |
| atlas | 128 | 0.9138 | 1.648 | 2.371 | 3.047 | 614 | 2957 | 3 |
| atlas | 256 | 0.9366 | 3.194 | 4.503 | 5.600 | 323 | 1848 | 3 |
| atlas | 512 | 0.9565 | 5.876 | 7.638 | 9.277 | 179 | 945 | 3 |
| atlas | 1024 | 0.9708 | 11.257 | 14.367 | 17.227 | 93 | 483 | 3 |
| hnswlib | 64 | 0.8884 | 0.365 | 0.582 | 0.735 | 2549 | 7380 | 3 |
| hnswlib | 128 | 0.9155 | 0.637 | 0.927 | 1.184 | 1529 | 4962 | 3 |
| hnswlib | 256 | 0.9379 | 1.210 | 1.668 | 2.117 | 829 | 2681 | 3 |
| hnswlib | 512 | 0.9578 | 2.359 | 3.172 | 3.838 | 432 | 1340 | 3 |
| hnswlib | 1024 | 0.9726 | 4.468 | 5.677 | 6.815 | 231 | 756 | 3 |
| qdrant | 64 | 0.9181 | 3.395 | 5.069 | 7.265 | 274 | 668 | 3 |
| qdrant | 128 | 0.9481 | 4.500 | 5.819 | 7.046 | 215 | 512 | 3 |
| qdrant | 256 | 0.9704 | 6.730 | 8.940 | 11.925 | 144 | 326 | 3 |
| qdrant | 512 | 0.9831 | 10.007 | 13.015 | 15.919 | 97 | 215 | 3 |
| qdrant | 1024 | 0.9879 | 15.087 | 18.980 | 22.048 | 65 | 137 | 3 |

### Filtered search at ef=512 (each engine's native filtered path)

| engine | selectivity | path | recall@10 | p50 ms | p95 ms | p99 ms | avg returned | leaks (200 q) |
|---|---|---|---|---|---|---|---|---|
| atlas | 10.0% | native filtered HNSW walk | 0.993 | 40.75 | 50.39 | 61.75 | 10.0 | 0 |
| atlas | 5.0% | native filtered HNSW walk | 0.996 | 63.55 | 76.89 | 88.89 | 10.0 | 0 |
| atlas | 1.0% | exact pre-filter scan over allowed ids | 0.999 | 2.77 | 3.74 | 4.32 | 10.0 | 0 |
| atlas | 0.1% | exact pre-filter scan over allowed ids | 0.999 | 1.29 | 1.87 | 2.11 | 10.0 | 0 |
| hnswlib | 10% | hnswlib native filtered walk | 0.994 | 22.61 | 27.79 | 35.02 | 10.0 | 0 |
| hnswlib | 5% | hnswlib native filtered walk | 0.997 | 37.92 | 47.20 | 55.17 | 10.0 | 0 |
| hnswlib | 1% | hnswlib native filtered walk | 0.999 | 106.83 | 133.90 | 158.17 | 10.0 | 0 |
| hnswlib | 0.1% | hnswlib native filtered walk | 0.999 | 323.19 | 391.93 | 442.97 | 10.0 | 0 |
| qdrant | 10% | qdrant native filtered search (payload index) | 0.997 | 6.84 | 9.14 | 11.56 | 10.0 | 0 |
| qdrant | 5% | qdrant native filtered search (payload index) | 0.998 | 3.81 | 5.16 | 7.18 | 10.0 | 0 |
| qdrant | 1% | qdrant native filtered search (payload index) | 0.999 | 1.74 | 2.41 | 2.91 | 10.0 | 0 |
| qdrant | 0.1% | qdrant native filtered search (payload index) | 0.999 | 1.18 | 1.53 | 2.06 | 10.0 | 0 |

### Build, memory, disk

| engine | build s | ingest vec/s | peak RSS after build MB | disk MB | notes |
|---|---|---|---|---|---|
| atlas | 805.9 | 360 | 1034 | 377.1 | embedded library via AnnIndex trait, no WAL, no API |
| hnswlib | 314.0 | 923 | 717 | 377.1 |  |
| qdrant | 206.7 | 1403 | nan | nan | vmhwm_qdrant_process: VmHWM:	 1487684 kB VmRSS:	  797248 kB  storage_bytes: 672096482 version: qdrant 1.17.1 |

## sift

### Unfiltered recall@10 vs latency (median of repetitions, sequential single client)

| engine | ef | recall@10 | p50 ms | p95 ms | p99 ms | QPS seq | QPS 8 workers | reps |
|---|---|---|---|---|---|---|---|---|
| atlas | 64 | 0.9811 | 0.430 | 0.655 | 0.904 | 2544 | 11706 | 2 |
| atlas | 128 | 0.9952 | 0.733 | 1.246 | 1.759 | 1350 | 7226 | 2 |
| atlas | 256 | 0.9984 | 1.285 | 2.122 | 2.827 | 809 | 3751 | 2 |
| atlas | 512 | 0.9991 | 2.348 | 4.161 | 5.723 | 411 | 2337 | 2 |
| atlas | 1024 | 0.9992 | 4.013 | 6.566 | 8.548 | 246 | 1157 | 2 |
| hnswlib | 64 | 0.9810 | 0.190 | 0.301 | 0.389 | 4894 | 11901 | 2 |
| hnswlib | 128 | 0.9951 | 0.323 | 0.452 | 0.564 | 3036 | 8078 | 2 |
| hnswlib | 256 | 0.9985 | 0.575 | 0.811 | 1.022 | 1719 | 4420 | 2 |
| hnswlib | 512 | 0.9992 | 1.062 | 1.791 | 2.478 | 901 | 3165 | 2 |
| hnswlib | 1024 | 0.9993 | 1.922 | 3.166 | 3.949 | 503 | 1766 | 2 |
| qdrant | 64 | 0.9963 | 2.163 | 3.094 | 3.996 | 437 | 1090 | 2 |
| qdrant | 128 | 0.9992 | 2.886 | 3.934 | 4.914 | 337 | 935 | 2 |
| qdrant | 256 | 0.9996 | 3.764 | 5.316 | 6.740 | 256 | 791 | 2 |
| qdrant | 512 | 0.9996 | 5.159 | 7.238 | 9.186 | 190 | 557 | 2 |
| qdrant | 1024 | 0.9996 | 7.415 | 10.222 | 12.827 | 133 | 346 | 2 |

### Filtered search at ef=512 (each engine's native filtered path)

| engine | selectivity | path | recall@10 | p50 ms | p95 ms | p99 ms | avg returned | leaks (200 q) |
|---|---|---|---|---|---|---|---|---|
| atlas | 10.0% | native filtered HNSW walk | 1.000 | 15.48 | 21.34 | 24.69 | 10.0 | 0 |
| atlas | 5.0% | native filtered HNSW walk | 1.000 | 24.86 | 33.64 | 41.76 | 10.0 | 0 |
| atlas | 1.0% | exact pre-filter scan over allowed ids | 1.000 | 6.62 | 8.98 | 9.77 | 10.0 | 0 |
| atlas | 0.1% | exact pre-filter scan over allowed ids | 1.000 | 4.28 | 6.38 | 7.19 | 10.0 | 0 |
| hnswlib | 10% | hnswlib native filtered walk | 1.000 | 12.84 | 20.52 | 25.07 | 10.0 | 0 |
| hnswlib | 5% | hnswlib native filtered walk | 1.000 | 18.44 | 24.76 | 32.83 | 10.0 | 0 |
| hnswlib | 1% | hnswlib native filtered walk | 1.000 | 74.08 | 112.59 | 147.77 | 10.0 | 0 |
| hnswlib | 0.1% | hnswlib native filtered walk | 1.000 | 611.16 | 874.08 | 1002.66 | 10.0 | 0 |
| qdrant | 10% | qdrant native filtered search (payload index) | 1.000 | 4.51 | 6.03 | 7.59 | 10.0 | 0 |
| qdrant | 5% | qdrant native filtered search (payload index) | 1.000 | 4.38 | 6.57 | 7.63 | 10.0 | 0 |
| qdrant | 1% | qdrant native filtered search (payload index) | 1.000 | 2.29 | 3.33 | 4.33 | 10.0 | 0 |
| qdrant | 0.1% | qdrant native filtered search (payload index) | 1.000 | 1.17 | 1.46 | 1.66 | 10.0 | 0 |

### Build, memory, disk

| engine | build s | ingest vec/s | peak RSS after build MB | disk MB | notes |
|---|---|---|---|---|---|
| atlas | 815.2 | 1227 | 1920 | 788.3 | embedded library via AnnIndex trait, no WAL, no API |
| hnswlib | 120.6 | 8292 | 1406 | 788.3 |  |
| qdrant | 332.3 | 3009 | nan | nan | vmhwm_qdrant_process: VmHWM:	 3268024 kB VmRSS:	 1332168 kB  storage_bytes: 1130491865 version: qdrant 1.17.1 |
