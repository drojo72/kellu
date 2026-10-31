<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Kellu: estado y decisiones (traspaso al proyecto Rust)

Fecha: 2026-10-31 (actualizado al cierre de M1; el traspaso original es del 2026-10-29). Kellu se
reescribe en **Rust puro**. Hay dos prototipos anteriores en GitHub, ambos como **referencia de
comportamiento** (no se mantienen):

- Rama `original-python`: prototipo de Python, hitos M0 a M2, 185 casos de prueba (`ruff` y
  `mypy --strict` en verde).
- Rama `kellu-prototype-ruby`: prototipo en Ruby 3.2+ con CLI Thor, gestor de keyring sobre
  `secret-tool` (Open3), configuración TOML con `keyring_service`, adaptadores de DeepSeek,
  OpenRouter, Google y Gemma con `Net::HTTP`, bitácora de sesiones, activos y guías de marca.

La rama `main` es Rust, con historia nueva. Se eligió Rust puro y no un híbrido Ruby con extensiones
Rust por: borrado garantizado de secretos en memoria (`zeroize`), una sola cadena de suministro y
ninguna frontera FFI con `unsafe`, un binario único y liviano para los muchos procesos LSP que lanza
Kate, y soporte de primera clase para sandbox y syscalls. Para traer a `main` solo lo reutilizable
del prototipo Ruby (marca y bitácora), sin su historia:

```
git checkout kellu-prototype-ruby -- assets docs/branding docs/bitacora
```

El encabezado de todos los `.md` apunta a `/assets/branding/svg/kellu-logo-color-*.svg`: sin ese
checkout el logo no se ve.

## 0. Qué cambia y qué no
- **No cambia:** el diseño completo (amenazas, autorización, sandbox, LSP, auditoría), el esquema
  de configuración TOML y su `defaults.toml`, el esquema del keyring y los niveles, los proveedores
  y rutas por defecto, la superficie de la CLI y sus códigos de salida, el formato de las sesiones,
  la marca, el plan de versiones y los hitos.
- **Cambia:** el lenguaje y todo lo que depende de él. Ya no hay `pyproject`, venv, `ruff`, `mypy`,
  `pytest` ni `pip-audit`. Ahora hay workspace de Cargo, `cargo fmt`, `clippy`, `cargo test`,
  `cargo deny` y `cargo audit`. "Biblioteca estándar primero" pasa a "dependencias mínimas,
  auditadas y justificadas una a una". La regla de capas la hace cumplir Cargo.
- **Estrategia:** primero **paridad** con el prototipo (M0 a M2 portados, con sus 137 funciones de
  prueba como especificación), luego trabajo nuevo desde M3. Ver `docs/PORTING.md`.
- **Verificado:** Rust 1.91 con clippy y rustfmt está disponible por apt y baja crates de
  crates.io; los nombres `kellu`, `kellu-core`, `kellu-common`, `kellu-cli`, `kellu-lsp`,
  `kellu-sandbox`, `kellu-providers` y `kellu-security` estaban libres en crates.io (2026-10-29).
  Por lo tanto se compila y se prueba lo que se entrega.

## 0.1 Lecciones del prototipo Ruby
**Conservar:**
- Listar claves parseando los bloques `[/N]` de `secret-tool search --all` y leyendo la línea
  `attribute.kellu`; nunca derivar el nombre desde la etiqueta, que es texto libre.
- Mapeo explícito de proveedor a clave (`keyring_service` en Ruby, `keyring_name` en Rust), pero
  **restringido al espacio `ai_*`** (nivel `provider`). Sin esa restricción, un `config.toml`
  manipulado podría leer una clave de `infra` y enviarla a un `base_url` ajeno.
- `Net::HTTP` no sigue redirecciones: mantener el rechazo explícito en Rust y fijar siempre timeouts.
- La prueba real con DeepSeek devolvió HTTP 402 (sin saldo): el router debe pasar al siguiente
  proveedor. Añadir esa prueba.
- La bitácora de sesiones en `docs/bitacora/` (hábito útil, se mantiene).
- Backlog de la sesión 03: opción `--provider` en la CLI, proveedor por defecto con saldo activo y
  alinear los identificadores de modelo (Gemini `gemini-flash-latest`, Gemma `gemma-4-26b-a4b-it`,
  OpenRouter `openrouter/auto` o consulta de `/models`).

**Evitar:**
- Secretos como argumento de línea de comandos (`secret set nombre "valor"`): leer de stdin o sin
  eco; `get` muestra el valor solo con una opción explícita.
- `secret-tool search` imprime los secretos: cuando se pueda, listar sin cargar valores (API D-Bus
  `SearchItems`).
- Afirmar en `SECURITY.md` controles que aún no existen, o citar la CVE-2026-87766 como si fuera de
  resolución de rutas (es de bubblewrap).
- Dos entradas `0.1.0` en el CHANGELOG, `srcet` en lugar de `srcset` en el HTML del encabezado, y
  listar Cloudflare R2 como proveedor de IA (es almacenamiento, nivel `infra`).

## 1. Qué es
Asistente de IA seguro y modular para el editor Kate. Un núcleo pequeño sin interfaz decide si hace
falta un modelo, redacta lo que sale de la máquina, enruta con respaldo entre proveedores, ejecuta
las herramientas que lanzan procesos en un sandbox y se comunica con el editor mediante frontends
delgados (terminal, LSP; luego gateway, ACP y MCP). Desarrollo por hitos, seguridad primero.

## 2. Decisiones vigentes (no reabrir sin motivo)
| Tema | Decisión |
|---|---|
| Nombre | **Kellu**, "ayudante" en mapudungun (confirmado por el usuario). No usar "Keysign" (el holding) en nombres de producto ni "Kate" en el nombre. Kallu significa "azul": la grafía KALLU en un README fue un error. |
| Licencia | MIT. Falta rellenar `<copyright holder>` en `LICENSE`. |
| Lenguaje | **Rust**, edición 2024, MSRV 1.91, toolchain fijada en 1.91.1 (`rust-toolchain.toml`, M0). Núcleo con dependencias mínimas; `unsafe_code = "forbid"` y deny de `unwrap`, `expect` y `panic!` fuera de tests, ambos en `[workspace.lints]` y verificados por un test; secretos en tipos que se borran de memoria y no se imprimen (`secrecy`, desde M2). C++ solo para un posible plugin de Kate, que ya cubren KateAI y compañía vía ACP o gateway. |
| Workspace | `crates/`: `kellu-common`, `kellu-security`, `kellu-sandbox`, `kellu-providers`, `kellu-projects`, `kellu-tools`, `kellu-core`, `kellu-frontends`, más el binario `kellu`. Dirección: `frontends -> core -> tools -> sandbox -> security -> common`; `providers` y `projects` también bajo `core`. Un crate se crea cuando empieza su hito; un test sobre `cargo metadata` lo exige. Ver la arista `sandbox -> security` en la sección 9. |
| Configuración | TOML en `~/.config/kellu/config.toml` sobre `defaults.toml` embebido (`include_str!`). Sin secretos, https obligatorio (http solo localhost), claves desconocidas son error, archivo propio y sin escritura de grupo u otros, máximo 256 KiB. |
| Keyring | Atributo `kellu=<servicio>_<clave>` y etiqueta `KELLU: <nombre>` (mismo formato que el prototipo, que usaba el atributo `kai`). Niveles deducidos del nombre: `ai_` es `provider`; `github_`, `cloudflare_`, `render_`, `desec_`, `duckdns_` son `infra`; `neon_`, `supabase_`, `mysql_` son `db`; desconocido cae en `infra`. `keyring_name = "x"` resuelve a `ai_x_api_key`. En M3 se decide entre llamar a `secret-tool` (sin dependencia nueva, compatible) o un crate de Secret Service con atributos arbitrarios (permitiría `kellu keys list`). |
| Proveedores por defecto | openrouter, deepseek (compatibles con OpenAI); google (`gemini-flash-latest`, alias móvil); gemma_direct (`gemma-4-26b-a4b-it`, clave y cuota propias); grok (`grok-4.3`, base `https://api.x.ai/v1`). Google usa `v1beta`. Rutas: general = gemma_direct, google, deepseek, openrouter; code = deepseek, openrouter; explain = google, openrouter. Presupuesto total del router: 120 s. |
| HTTP | Candidato: `ureq` con `rustls`. Verificar en M2 que permita cero redirecciones, timeouts y lectura en streaming, que interprete las URL igual que `http::Uri` (la validación de M1 usa ese parser) y decidir las raíces de certificados. |
| Sandbox | bubblewrap con `argv` como lista, sin shell. Política de versión **B**: se permiten versiones anteriores a 0.12.0 con advertencia (CVE-2026-87766), porque el diseño nunca crea puntos de montaje dentro del árbol del proyecto; modo estricto opcional. Ubuntu 24.04 trae 0.9.0 y su paquete retiró el parche en una actualización. Landlock (crate `landlock`) como respaldo posterior. |
| Autorización | Lectura por proyecto (solo lectura). Escritura nunca permanente: el modelo propone diffs con hashes, el usuario elige denegar, parcial o completo, y **solo el núcleo aplica**. Ningún sandbox escribe en el proyecto real; las herramientas que escriben trabajan sobre una copia (staging) y su resultado vuelve como otra propuesta. Un solo `Grant` deriva la política de rutas y los montajes (prueba con `proptest`). Auditoría JSONL append-only (0600) con cadena de hashes. Acceso a archivos por descriptores (`cap-std` o `rustix openat2`). |
| LSP en Kate | 0.1.0: hover y completion manual. 0.2.0: diagnósticos y code actions (una acción por hunk más "aplicar todo"). Chat, vista previa de diffs y texto fantasma necesitan otros frontends. Raíz igual a `$HOME`, sus ancestros o `/` se rechaza. La configuración de `.kateproject` es no confiable y solo puede endurecer. Candidato: `lsp-server` y `lsp-types`. |
| Marca | Azul marino `#14253D`, dorado `#C8A66A`, cobre `#B87355`, marfil `#F4F0E7`. Símbolo: geometría abstracta basada en figuras de textiles mapuches. Pendiente: versión simplificada para 16 y 24 px, rediseño vectorial propio, tipografía libre modificada por el usuario con registro de licencia (`docs/brand/PROVENANCE.md`). Dorado sobre marfil no alcanza contraste (2.0:1): usar un dorado más oscuro en fondos claros. Terminal: niveles ASCII `<*>`, Unicode `‹ ◆ ›`, color; respetar `NO_COLOR`, `TERM=dumb` y locale no UTF-8 (`kellu-frontends`, módulo `theme`, M6). |
| Encabezado de los `.md` | Todos los `.md` llevan el mismo encabezado con el logo (`/assets/branding/svg/kellu-logo-color-{dark,light}.svg`) y el lema "Local-first, modular AI assistant & LSP CLI built with Ruby & Rust". Ver la nota sobre el lema en la sección 9. |

## 3. Hitos
| Hito | Prototipo Python (`original-python`) | Rust (`main`) |
|---|---|---|
| M0 | Hecho | **Hecho.** Workspace, toolchain fijada, `deny.toml`, CI, `kellu --version`, `doctor`, rutas XDG, archivos privados, errores, test de arquitectura |
| M1 | Hecho | **Hecho (paridad).** Config TOML estricta, logging privado con gancho de redacción, sesiones, `config path|check|show`, comprobación de config en `doctor` |
| M2 | Hecho | **Siguiente (paridad).** Proveedores (OpenAI-compatible con streaming y herramientas, Google), transporte HTTP endurecido, router con respaldo |
| M3 | No empezado | Vault de keyring con niveles, `kellu keys list|set|rm|cleanup-legacy`, entorno depurado, deny globs, redactor v1, permisos por riesgo, scope y `Grant`, auditoría, `kellu providers test` |
| M4 | No empezado | Sandbox: política derivada del `Grant`, copia de trabajo, límites de recursos, sondeo de bubblewrap en `doctor`, tests de escape |
| M5 | No empezado | Registro de herramientas con riesgo obligatorio, propuesta y aplicación de diffs, archivos, shell en sandbox, git de lectura; escrituras de git y gh en el host con confirmación |
| M6 | No empezado | REPL, LSP (hover y completion), tema de terminal |
| M7 | No empezado | Documentación y etiqueta 0.1.0 |
El prototipo Ruby avanzó en paralelo en keyring (altas, bajas, lectura y listado), configuración y
adaptadores, así que es una segunda referencia para M2 y M3.

Después: 0.2.0 agente (loop con herramientas, diagnósticos y code actions); 0.3.0 gateway
compatible con OpenAI y ACP; 0.4.0 memoria (SQLite, pgvector opcional); 0.5.0 runtimes locales,
perfiles de hardware y MCP.

## 4. Inventario de claves (solo nombres)
- provider: `ai_openrouter_api_key`, `ai_google_api_key`, `ai_deepseek_api_key`, `ai_grok_api_key`, `ai_gemma_direct_api_key` (ya copiada; borrar la entrada antigua `gemma_direct_api_key` tras verificar).
- infra: `github_token`, `cloudflare_tunnel_token`, `cloudflare_workers_token`, `cloudflare_r2_token`, `cloudflare_pages_token`, `cloudflare_api_token`, `render_api_key`, `desec_token`.
- db: `neon_admin_key`, `supabase_admin_key`.
- Borrar: `test_key`. No migradas (probablemente nunca guardadas): `cloudflare_zone_token`, `duckdns_token`, `mysql_*`.
- Siguen existiendo las entradas del prototipo bajo el atributo `kai`; borrarlas cuando Kellu lea bien (`kellu keys cleanup-legacy`, M3).
- Abiertas: revocar o reducir `cloudflare_api_token` (token completo, ya hay acotados) y rotar el PAT de GitHub por uno fine-grained sin `delete_repo`.
- `secret-tool search` imprime los secretos: filtrar con `grep -E '^(label|attribute\.)'`. Nunca pegar valores en el chat.

## 5. Pendientes del usuario
- Crear las ramas (comandos arriba). Considerar reservar `kellu` en crates.io.
- Rellenar `<copyright holder>` (LICENSE y `authors` en `Cargo.toml`), `<your-user>` (READMEs y `repository` en `Cargo.toml`) y el contacto de seguridad de `SECURITY.md`.
- Fijar las acciones de GitHub por SHA antes de publicar (el CI de Rust ya existe; falta ejecutarlo en GitHub por primera vez).
- Traer los activos de marca (`git checkout kellu-prototype-ruby -- assets docs/branding docs/bitacora`) para que el logo del encabezado se vea.
- Borrar `test_key` y la entrada antigua de gemma_direct.
- Confirmar en su cuenta el identificador de modelo de Grok y que los alias de Gemini respondan.
- Probar en su Kate: si las code actions aparecen sin diagnóstico, `showMessage` y `$/progress`, y si `.kateproject` puede sobrescribir el comando del servidor.
- Marca: versión de 16 px, vectorización, fuente libre y su registro de licencia.

## 6. Cómo trabajar
- Entorno de verificación (se repite en cada sesión; no persiste):
  `apt-get install -y cargo-1.91 rustc-1.91 rust-1.91-clippy rustfmt-1.91 bubblewrap` y
  `export PATH=/usr/lib/rust-1.91/bin:$PATH`.
- Antes de entregar: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked`; `cargo deny check` y `cargo audit` cuando se puedan instalar
  (`cargo install --locked cargo-deny cargo-audit --root /tmp/tools`, tarda varios minutos).
  Decir explícitamente qué no se pudo verificar.
- Tests negativos de seguridad para todo lo sensible (symlinks, permisos, inyección, secretos en
  errores). Cada función de `docs/porting/python-tests.txt` necesita su equivalente en Rust o una
  razón escrita (registro en `docs/porting/PARITY.md`).
- Entregar el repositorio como zip más los documentos que cambien. Los archivos del sandbox del chat
  no persisten: el estado vive en el zip.
- Respuestas en español, breves y pragmáticas; código, documentación y commits en inglés.

## 7. No verificado
Llamadas reales a las APIs de los proveedores; las capacidades exactas de `ureq` y `rustls` para
nuestras reglas; comportamiento de bubblewrap como usuario sin privilegios en el equipo del usuario
(las pruebas se hicieron como root); los comportamientos de Kate señalados en la sección 5; `cargo
audit` (no se pudo instalar a tiempo); el CI en GitHub (no se ha ejecutado); las pruebas de
"archivo o directorio de otro usuario" solo corren cuando los tests se ejecutan como root.

## 8. Archivos de este traspaso
`HANDOFF.md`, `README.md`, `README.es.md`, `SECURITY.md`, `CHANGELOG.md`, `LICENSE`,
`docs/ARCHITECTURE.md`, `docs/PORTING.md`, `docs/porting/python-tests.txt`, `docs/porting/PARITY.md`,
`docs/design/permissions-and-sandbox.md`, `docs/design/lsp-capabilities.md` y el workspace de Cargo completo (`kellu-M1.zip`). `kellu-project-instructions.md` vive como
instrucciones del proyecto y no está en el repositorio. El prototipo de Python completo
(`kellu-M2.zip`) es para la rama `original-python`.

## 9. Estado al cierre de M1 (2026-10-31)

**Verificado:** `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
y `cargo test --workspace --locked` (147 tests) en verde con Rust 1.91.1; `cargo deny check` en
verde (al cierre de M0). **No verificado:** ver sección 7.

**Hecho en M0 y M1:** 80 de las 137 funciones de prueba de Python tienen equivalente o razón escrita
(ver `PARITY.md`); el binario `kellu` con `--version`, `doctor` y `config path|check|show`; los
crates `kellu-common` (app, error, net, paths, text, utc), `kellu-core` (config, log, session) y
`kellu-providers` (solo tipos de configuración).

**Decisiones tomadas en M0 y M1 (con motivo):**
- Edición 2024, MSRV 1.91, toolchain 1.91.1.
- Sin `anyhow`: el binario usa `KelluError`; una dependencia menos.
- `http` en lugar de `url` para validar endpoints: es el parser que usará el cliente HTTP (sin
  diferencial entre validar y conectar) y evita arrastrar el árbol de ICU.
- `toml` leído como tabla y validado a mano, sin `serde` derive: los errores nombran la ruta de la
  clave y nunca repiten valores.
- Hora propia en UTC (`kellu_common::utc`) en lugar del crate `time`: no hace falta la base de zonas
  horarias. **Desviación de paridad:** el prototipo usaba hora local; los nombres de sesión y los
  encabezados ahora muestran UTC.
- Arista permitida `sandbox -> security` (los montajes se derivan del `Grant`, que vive en
  `kellu-security`). El diagrama del traspaso original no la mostraba. La alternativa es mover la
  derivación Grant a montajes a `tools`.
- `projects` depende solo de `common`, y `tools` no depende de `projects` (el scope y los deny globs
  viven en `security`).
- La redacción es un gancho (`Redactor`): hasta M3 no hace nada, así que **los logs y las sesiones
  todavía no eliminan secretos**. `SECURITY.md` lo dice.
- Tests del `log` global en su propio proceso (`kellu-core/tests/logging.rs`), porque la fachada
  `log` tiene un único logger global.

**Nota sobre el lema del encabezado:** el lema pedido dice "built with Ruby & Rust", pero el
proyecto es Rust puro (el prototipo Ruby es solo referencia). Se aplicó tal cual; conviene decidir si
se cambia a "built in Rust" antes de publicar.

**Siguiente: M2 (paridad).** Orden sugerido:
1. Verificar `ureq` + `rustls` contra las reglas (cero redirecciones, timeouts, streaming, tope de
   cuerpo de 8 MiB y de línea SSE de 1 MiB, parseo de URL igual que `http::Uri`).
2. `secrecy`/`zeroize` para las claves; `serde` y `serde_json` para las cargas de proveedores.
3. `kellu-providers`: trait `Provider`, cliente OpenAI-compatible (streaming y herramientas), Google
   (Gemini y Gemma), registro de tipos (el test `provider_kinds_match_the_registry` debe seguir
   verde) y errores con `retryable`.
4. Servidor de prueba con guion (`tiny_http` o `TcpListener`) y las 57 funciones pendientes del
   registro (26 OpenAI, 12 Google, 19 router), incluida la nueva prueba de HTTP 402 con respaldo.
5. `kellu-core: router` con `classify_task` (decidir plegado de acentos) y presupuesto de tiempo; la
   salida de `config show` ya lista proveedores y rutas.
