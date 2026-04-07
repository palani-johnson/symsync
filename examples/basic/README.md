# Basic example

| name                                                            | type    | target                                                             |
| --------------------------------------------------------------- | ------- | ------------------------------------------------------------------ |
| .symsync.state                                                  | dir     |                                                                    |
| .symsync.state/sources                                          | dir     |                                                                    |
| .symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 | symlink | ../../file_0.txt                                                   |
| .symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f | symlink | ../../file_2.txt                                                   |
| .symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 | symlink | ../../file_1.txt                                                   |
| .symsync.state/targets.toml                                     | file    |                                                                    |
| README.md                                                       | file    |                                                                    |
| file_0.txt                                                      | file    |                                                                    |
| file_1.txt                                                      | file    |                                                                    |
| file_2.txt                                                      | file    |                                                                    |
| out                                                             | dir     |                                                                    |
| out/file_0.txt                                                  | symlink | ../.symsync.state/sources/064525aa0ae4a2240df5e54f08c3004cbe29cba1 |
| out/file_1.txt                                                  | symlink | ../.symsync.state/sources/bb862d7ccad0c082b3db9f05b57d5f0384eb9471 |
| out/file_2.txt                                                  | symlink | ../.symsync.state/sources/2b562f4c942af3f299539c934e5d23914c1cd43f |
| symsync.toml                                                    | file    |                                                                    |

Table generated with the `nushell` command:

```nu
ls **/* -la | select name type target | to md
```
