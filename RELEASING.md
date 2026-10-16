# Fluxo de trabalho e releases

## Todo código entra por PR

A branch `main` é protegida: não aceita push direto (nem de admin) e só faz merge com o CI verde
(`fmt + clippy` e `build + test` em Linux, macOS e Windows).

```bash
git switch -c minha-mudanca
# … commits …
git push -u origin minha-mudanca
gh pr create --fill
```

Depois do merge, volte para a `main` atualizada:

```bash
git switch main
git pull
```

## Lançar uma versão

1. Num PR, atualize `version` em `client/Cargo.toml` (ex.: `0.2.0`) e rode `cargo build` para atualizar o
   `Cargo.lock`. Faça o merge.
2. Na `main` atualizada, crie e envie a tag. Ela **precisa** bater com a versão do `Cargo.toml`:

   ```bash
   git tag v0.2.0
   git push origin v0.2.0
   ```

3. O workflow [`release.yml`](.github/workflows/release.yml):
   - confere se a tag bate com a versão do `Cargo.toml` (se não bater, para sem publicar nada);
   - roda os testes e compila CLI (`ampero`) e GUI (`ampero-gui`) para cada plataforma;
   - publica a Release `Ampero vX.Y.Z` com notas geradas a partir dos PRs e os pacotes para download.

| Pacote | Conteúdo |
|---|---|
| `ampero-X.Y.Z-windows-x64-setup.exe` | instalador do Windows (por usuário, sem admin; menu Iniciar e atalho opcional) |
| `ampero-X.Y.Z-windows-x64.zip` | `ampero.exe`, `ampero-gui.exe` |
| `ampero-X.Y.Z-linux-x64.tar.gz` | `ampero`, `ampero-gui` (precisa de ALSA e X11/Wayland) |
| `ampero-X.Y.Z-macos-universal.tar.gz` | `ampero`, `ampero-gui` (Intel e Apple Silicon no mesmo binário) |
| `SHA256SUMS` | checksums de todos os pacotes |

### Atualização pelo próprio app

Ao abrir, a GUI consulta a última release no GitHub. Se houver versão mais nova, aparece uma faixa com
"Novidades" e **Atualizar e reiniciar**: o app baixa `ampero-X.Y.Z-windows-x64-setup.exe`, confere o SHA-256 com o
`SHA256SUMS` da release, roda o instalador em silêncio (`/RESTARTAPP=1`) e fecha; o instalador abre a versão nova.
Isso só vale para a cópia instalada pelo instalador (o app confere o `InstallLocation` no registro). Cópias do zip,
no macOS e no Linux recebem um botão para a página da release. O menu ⚙ também tem "Verificar atualizações".

Para testar o instalador localmente: `powershell -ExecutionPolicy Bypass -File client/installer/build.ps1`
(precisa do Inno Setup: `winget install -e --id JRSoftware.InnoSetup`).

Versionamento: [SemVer](https://semver.org/lang/pt-BR/). Use `0.x.y` enquanto o protocolo e a interface ainda mudam.

### Observações

- **macOS:** os binários não são assinados nem notarizados. Na primeira execução, abra pelo menu de contexto
  (botão direito → Abrir) ou rode `xattr -d com.apple.quarantine ampero-gui`.
- **Windows:** o SmartScreen pode avisar na primeira execução (binário não assinado). Clique em "Mais informações →
  Executar assim mesmo".
- Fotos e descrições dos efeitos não vão nos pacotes; elas são lidas da instalação do Ampero Editor oficial
  (veja `client/README.md`).
- Para refazer uma release que falhou, apague a release e a tag e envie a tag de novo:
  `gh release delete vX.Y.Z --cleanup-tag`, depois `git tag -f vX.Y.Z && git push origin vX.Y.Z`.
