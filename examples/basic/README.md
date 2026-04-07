# Basic example

| name                                                            | type    | target                                                             |
| --------------------------------------------------------------- | ------- | ------------------------------------------------------------------ |
| .symsync.state                                                  | dir     |                                                                    |
| .symsync.state/sources                                          | dir     |                                                                    |
| .symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 | symlink | ../../file_0.txt                                                   |
| .symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 | symlink | ../../file_1.txt                                                   |
| .symsync.state/sources/f0954953a4ad5e0595332ada60f1a4d29e5cc49f | symlink | ../../file_3.txt                                                   |
| .symsync.state/targets.toml                                     | file    |                                                                    |
| README.md                                                       | file    |                                                                    |
| file_0.txt                                                      | file    |                                                                    |
| file_1.txt                                                      | file    |                                                                    |
| file_3.txt                                                      | file    |                                                                    |
| out                                                             | dir     |                                                                    |
| out/file_0.txt                                                  | symlink | ../.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 |
| out/file_1.txt                                                  | symlink | ../.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 |
| out/file_3.txt                                                  | symlink | ../.symsync.state/sources/f0954953a4ad5e0595332ada60f1a4d29e5cc49f |
| symsync.toml                                                    | file    |                                                                    |

Table generated with the `nushell` command:

```nu
ls **/* -la | select name type target | to md
```
