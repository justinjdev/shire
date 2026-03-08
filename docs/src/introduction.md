# Shire

<div align="center">
<pre>
                       .,:lccc:,.
                  .,codxkkOOOOkkxdoc,.
              .;ldkkOOOOOOOOOOOOOOOkkdl;.
           .:oxOOkxdollccccccccllodxkOOkxo:.
         ,lkOOxl;..                ..,lxOOkl,
       .ckOOd:.                        .:dOOkc.
      ;xOOo,          .,clllc,.          ,oOOx;
     lOOk;         .:dkOOOOOOkd:.         ;kOOl
    oOOx,        .ckOOOOOOOOOOOOkc.        ,xOOo
   lOOk,        ;xOOOkdl:;;:ldkOOOx;        ,kOOl
  ;OOO;        lOOOd;.        .;dOOOl        ;OOO;
  dOOd        :OOOl              lOOO:        dOOd
  kOOl        oOOx      .;;.     xOOo        lOOk
  kOOl        oOOx     .xOOx.    xOOo        lOOk
  dOOd        :OOOl    .oOOo.   lOOO:        dOOd
  ;OOO;        lOOOd;.  .,,. .;dOOOl        ;OOO;
   lOOk,        ;xOOOkdl:,:ldkOOOx;        ,kOOl
    oOOx,        .ckOOOOOOOOOOOOkc.        ,xOOo
     lOOk;         .:dkOOOOOOkd:.         ;kOOl
      ;xOOo,          .,clllc,.          ,oOOx;
       .ckOOd:.                        .:dOOkc.
         ,lkOOxl;..                ..,lxOOkl,
           .:oxOOkxdollccccccccllodxkOOkxo:.
              .;ldkkOOOOOOOOOOOOOOOkkdl;.
                  .,codxkkOOOOkkxdoc,.
                       .,:lccc:,.
</pre>
</div>

*One index to rule them all.*

**S**earch, **H**ierarchy, **I**ndex, **R**epo **E**xplorer — a monorepo package indexer that builds a dependency graph in SQLite and serves it over [Model Context Protocol](https://modelcontextprotocol.io/).

Point it at a monorepo. It discovers every package, maps their dependency relationships, extracts symbols from source code, and gives your AI tools structured access to the result.

## Get started in 30 seconds

```sh
brew install justinjdev/shire/shire
shire init --global
shire build
```

That's it. Claude Code can now search your packages, symbols, files, and dependency graph. See [Setup](./setup.md) for details.
