# Zed package and Git submodule interoperability

`ftnl-monorepo` owns application repositories through real Git submodules under `apps/`. It does not duplicate those repositories in `.zpkg.toml` dependencies.

## Supported workflows

```bash
git clone --recurse-submodules https://github.com/file-tunnel/ftnl-monorepo.git
zed install --git-submodules
```

For an existing checkout whose dependencies need to become submodules:

```bash
zed overtake --git-submodules
```

The transition must preserve `.gitmodules`, gitlink mode `160000`, and the selected submodule commit. A repository may be owned by a submodule or a Zed dependency, never both.

## Deliberate exclusions

`file-tunnel/ftnl-cli` and `file-tunnel/ftnl-infra` are not monorepo dependencies and are not submodules. The CLI consumes the interfaces and clients packages externally; infrastructure remains independently deployable.
