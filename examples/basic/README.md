# Basic example

| name                                                            | type    | target                                                             |
| --------------------------------------------------------------- | ------- | ------------------------------------------------------------------ |
| .symsync.state                                                  | dir     |                                                                    |
| .symsync.state/sources                                          | dir     |                                                                    |
| .symsync.state/sources/5436437fa01a7d3e41d46741da54b451446774ca | symlink | ../../file.txt                                                     |
| .symsync.state/sources/c2edf7b002d0354039a8aaba3bc53180caf3d248 | symlink | ../../file2.txt                                                    |
| .symsync.state/targets.toml                                     | file    |                                                                    |
| file.txt                                                        | file    |                                                                    |
| file2.txt                                                       | file    |                                                                    |
| out                                                             | dir     |                                                                    |
| out/file.txt                                                    | symlink | ../.symsync.state/sources/5436437fa01a7d3e41d46741da54b451446774ca |
| out/file2.txt                                                   | symlink | ../.symsync.state/sources/c2edf7b002d0354039a8aaba3bc53180caf3d248 |
| symsync.toml                                                    | file    |                                                                    |

Generated with the nushell command:

```nu
ls **/* -la | select name type target | to md
```
