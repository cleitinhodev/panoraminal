<p align="center">
  <img src="crates/panoraminal/assets/icons/panoraminal_icon_256.png" width="128" alt="Ícone do Panoraminal">
</p>

<h1 align="center">Panoraminal</h1>

<p align="center">
  <b>Pinte panoramas 360° por dentro.</b><br>
  Um programa de pintura nativo, acelerado por GPU, para imagens equirretangulares — feito em Rust com wgpu e egui.
</p>

<p align="center">
  <a href="https://github.com/cleitinhodev/panoraminal/releases/latest">Baixar</a> ·
  <a href="https://www.bugzinho.com/">Site</a> ·
  <a href="https://www.bugzinho.com/donate">Doações</a> ·
  <a href="README.md">English</a>
</p>

![Panoraminal — tema claro, vista esférica](docs/screenshots/light-sphere.png)

## O que é?

O Panoraminal permite pintar **diretamente na esfera** de um panorama 360°: você fica no centro, olha em volta e pinta como se a tela estivesse ao seu redor. Cada pincelada é projetada de volta na imagem equirretangular com a distorção correta — as linhas ficam retas na esfera, a emenda é invisível e os polos se comportam.

No final, exporte um **JPEG 360° com metadados GPano**, reconhecido pelo Facebook, Google Fotos e visualizadores de VR, ou um PNG comum.

É o sucessor do protótipo *Paint360*, reescrito do zero como um aplicativo de desktop de verdade, com motor de renderização próprio.

## Recursos

- **Duas vistas** — *Esfera* (câmera em perspectiva, campo de visão até 120°) e *Plana* (equirretangular com prévia da emenda). Alterne com `V`.
- **Motor de pincel na esfera** — caneta, pincel, aerógrafo e borracha; tamanho, opacidade, fluxo, dureza, espaçamento, suavização e correção polar.
- **Pincéis personalizados** (novo na 1.1) — biblioteca de pincéis com miniaturas, traço de prévia ao vivo, renomear, categorias, favoritos e busca. Importe **`.abr` do Photoshop** (v1/v2 e v6+, pontas amostradas com suas predefinições), **`.gbr` / `.gih` do GIMP** e qualquer **PNG / JPEG / WebP** como ponta. Dinâmica: espaçamento, ângulo, arredondamento, variação de tamanho/ângulo/arredondamento, dispersão, quantidade, variação de opacidade/fluxo, espelhamentos aleatórios e pressão da caneta. Os carimbos são projetados na esfera, então ficam corretos na emenda e nos polos. Os pincéis usados num projeto são incorporados ao arquivo `.pano`.
- **Posicionar imagens na cena** (novo na 1.1) — solte, cole ou insira uma foto/logo e ela flutua sobre o panorama como um objeto não destrutivo: mova, escale (uniforme ou livre), gire, espelhe e ajuste a opacidade com alças na tela, nas duas vistas. Duas projeções: **Envolver** (curvada sobre a esfera como um adesivo) e **Persistente** (um quadro plano projetado do centro, sem distorção a partir da direção ancorada, com *Reapontar para a vista*). Prévia ao vivo na GPU, aplicação em qualidade total com `Enter`, *Posicionar como nova camada*, `Esc` para cancelar, desfazer/refazer.
- **Pressão da caneta** no tamanho e/ou na opacidade (veja as [limitações](#limitações-conhecidas)).
- **Linhas retas** com `Shift`+clique (círculos máximos), **conta-gotas** (`I` ou `Alt`+clique) e **balde de tinta 360°** que atravessa a emenda.
- **Camadas** — até 64, reordenáveis por arrastar e soltar, renomear, visibilidade, trava de transparência, opacidade e 9 modos de mesclagem (normal, multiplicar, tela, sobrepor, escurecer, clarear, adição/brilho, diferença, luz suave), com miniaturas ao vivo.
- **Desfazer / refazer** com limite de memória configurável.
- **Projetos `.pano`** (ZIP com camadas PNG + manifesto JSON + prévia). Projetos antigos do Paint360 (`.p360.json`) são importados.
- **Importar** PNG, JPEG, WebP, BMP e TIFF como novo documento ou como camada; arraste arquivos para a janela.
- **Exportar** PNG ou JPEG 360° (XMP GPano), girar o rumo e espelhar horizontalmente.
- **Salvamento automático e recuperação** após falhas, aviso de alterações não salvas.
- **Temas** — escuro, claro ou do sistema, com paleta tirada do ícone. Gradientes, brilho neon e animações podem ser reduzidos ou **desligados** (Configurações → Aparência); a preferência do sistema de "reduzir movimento" é respeitada.
- Janelas de **Ajuda** (todos os atalhos, ferramentas e dicas de 360°) e **Sobre**.
- **8 idiomas** — Português (Brasil), English, Español, Français, Deutsch, Italiano, 日本語, 简体中文. Detectado automaticamente e trocável em tempo real; os caracteres CJK vêm embutidos.
- Diálogos de arquivo nativos, associação de arquivos `.pano` (instalador do Windows, `.desktop`/MIME no Linux).

| | |
|---|---|
| ![Biblioteca de pincéis](docs/screenshots/custom-brushes-light-pt.png) | ![Imagem posicionada (projeção Persistente)](docs/screenshots/place-persistent.png) |
| ![Imagem posicionada (projeção Envolver)](docs/screenshots/place-wrap.png) | ![Pincéis personalizados](docs/screenshots/custom-brushes.png) |
| ![Tema escuro](docs/screenshots/dark-sphere.png) | ![Vista plana](docs/screenshots/dark-flat.png) |
| ![Janela de ajuda](docs/screenshots/help-dark.png) | ![Janela Sobre](docs/screenshots/about-light.png) |
| ![Configurações em japonês](docs/screenshots/settings-ja.png) | |

## Download

Baixe a versão mais recente na [página de Releases](https://github.com/cleitinhodev/panoraminal/releases/latest):

| Plataforma | Arquivo |
|---|---|
| Windows 10/11 x64 | `Panoraminal-Setup-<versão>-x64.exe` (instalador) ou `Panoraminal-<versão>-windows-x64-portable.zip` |
| Linux x86_64 | `Panoraminal-<versão>-x86_64.AppImage`, `panoraminal_<versão>_amd64.deb` ou `panoraminal-<versão>-linux-x86_64.tar.gz` |
| macOS (universal) | `Panoraminal-<versão>-macos-universal.zip` (sem assinatura — na primeira vez, clique com o botão direito → Abrir) |

Recomenda-se uma GPU com Vulkan, Metal, DirectX 12 ou OpenGL 3.3+. O Panoraminal também roda em renderizadores por software (mais devagar).

## Atalhos de teclado

No macOS, `Ctrl` vira `⌘`. A lista completa está em **Ajuda → Atalhos** (`F1`).

| Ferramentas | | Pincel | |
|---|---|---|---|
| `P` Caneta | `B` Pincel | `[` / `]` Tamanho − / + | `{` / `}` Dureza − / + |
| `A` Aerógrafo · `C` Pincel personalizado | `E` Borracha | `Shift`+clique Linha a partir do último ponto | `Alt`+clique Pegar cor |
| `I` Conta-gotas | `F` Balde | `X` Trocar cores | `Esc` Cancelar traço / fechar janela |

| Arquivo | | Editar e camadas | |
|---|---|---|---|
| `Ctrl+N` Novo | `Ctrl+O` Abrir | `Ctrl+Z` Desfazer | `Ctrl+Shift+Z` / `Ctrl+Y` Refazer |
| `Ctrl+S` Salvar | `Ctrl+Shift+S` Salvar como | `Del` Limpar camada | `Ctrl+Shift+N` Nova camada |
| `Ctrl+E` Exportar PNG | `Ctrl+Shift+E` Exportar JPEG 360° | `Ctrl+J` Duplicar camada | `Ctrl+M` Mesclar abaixo |
| `Ctrl+I` Importar como camada | `Ctrl+,` Configurações | `Ctrl+Shift+V` / `Ctrl+V` Colar imagem | `Enter` Mesclar imagem posicionada |
| `Ctrl+Shift+I` Inserir imagem | `Ctrl+Shift+B` Importar pincéis | `Esc` Cancelar posicionamento | |

| Vista | |
|---|---|
| Arrastar com botão direito/meio ou `Espaço`+arrastar — olhar em volta | Roda do mouse, `+` / `-` — zoom (campo de visão) |
| Setas — girar a vista | `0` Redefinir vista |
| `Tab` Ocultar interface | `F11` Tela cheia |

## Compilando a partir do código

Requisitos: Rust estável recente (1.95 ou mais novo). No Linux também são necessários os pacotes de desenvolvimento do GTK 3 e do xkbcommon:

```sh
sudo apt install libgtk-3-dev libxkbcommon-dev   # Debian / Ubuntu
cargo run --release -p panoraminal
```

Testes e verificações:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Os scripts de empacotamento ficam em [`packaging/`](packaging): `packaging/linux/build-packages.sh` gera o AppImage, o `.deb` e o tarball; `packaging/windows/panoraminal.nsi` é o instalador NSIS. Ao enviar uma tag `v*`, o GitHub Actions compila todas as plataformas — veja [`.github/workflows/release.yml`](.github/workflows/release.yml).

### Estrutura do projeto

| Crate | Função |
|---|---|
| `panoraminal-core` | Documento, camadas, modos de mesclagem, matemática esfera ↔ equirretangular, motor de pincel, balde, histórico (sem GPU, com testes) |
| `panoraminal-render` | Renderizador wgpu: composição de camadas e shaders de projeção esférica / plana |
| `panoraminal-io` | Formato `.pano`, importação legada, importar/exportar imagens, XMP GPano |
| `panoraminal-i18n` | Carregamento de idiomas, fallback e placeholders |
| `panoraminal` | O aplicativo: interface egui, temas, ferramentas, configurações, ajuda/sobre, linha de comando |

### Linha de comando

```
panoraminal [ARQUIVO]              abre um projeto .pano ou uma imagem
  --lang <código>                  força um idioma (en, pt-BR, es, fr, de, it, ja, zh-CN)
  --theme <dark|light|system>      força um tema
  --tool <pen|brush|airbrush|custom|eraser>   começa com uma ferramenta
  --version, --help
```

## Limitações conhecidas

- A **pressão da caneta** vem do sistema via winit. Funciona com Windows Ink / canetas do tipo "pointer" e dispositivos com força de toque; drivers de mesa digitalizadora apenas Wintab e muitas mesas no X11/Wayland ainda não informam pressão ao winit — nesse caso o traço usa pressão máxima.
- A **importação de ABR** lê pontas amostradas e calculadas e as predefinições comuns (veja Recursos). Pincel duplo, textura, pontas de cerdas/erodíveis, dinâmica de cor, bordas molhadas e ruído ainda não têm equivalente: são ignorados e listados num aviso no pincel. Pontas coloridas viram máscaras (pintadas com a cor atual). O importador é testado com arquivos sintetizados; abra uma issue com qualquer `.abr` que não carregue.
- Imagens posicionadas são limitadas a 4096 px no lado maior (imagens maiores são reduzidas).
- Tamanho máximo do documento: 8192 × 4096 (limite de textura da maioria das GPUs); 64 camadas.
- A versão para macOS é gerada pelo CI e não é assinada nem notarizada.
- Com renderizador por software (llvmpipe, WARP), documentos grandes ficam lentos.
- Ao iniciar, o Panoraminal verifica se a API gráfica consegue criar um dispositivo e, se não, tenta a próxima (DirectX 12 → Vulkan → OpenGL no Windows, Vulkan → OpenGL no Linux). Para forçar uma, defina `WGPU_BACKEND=vulkan`, `dx12`, `gl` ou `metal`.

## Traduções

Os arquivos de idioma são JSON simples em [`crates/panoraminal-i18n/locales`](crates/panoraminal-i18n/locales). Um teste garante que todo idioma tem todas as chaves e os mesmos `{placeholders}`. Veja o [CONTRIBUTING.md](CONTRIBUTING.md).

## Apoie o projeto

O Panoraminal é gratuito e de código aberto. Se ele for útil para você, considere [fazer uma doação](https://www.bugzinho.com/donate) — e visite [bugzinho.com](https://www.bugzinho.com/).

## Licença

[MIT](LICENSE) © 2026 cleitinhodev / bugzinho. O subconjunto da fonte Noto Sans CJK incluído é licenciado sob a [SIL Open Font License 1.1](crates/panoraminal/assets/fonts/OFL.txt).
