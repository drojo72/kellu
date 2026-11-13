<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
</p>
<p align="center">
    <em>Local-first, modular AI assistant & LSP CLI built with Rust</em>
</p>

---

# Kellu: estado y decisiones (traspaso al proyecto Rust)

Fecha: 2026-11-09 (M6 cerrado en la rama `m6-frontends`, desde `m5-tools`; M5 en `m5-tools`, probado por el usuario; M4 integrado en `main`; el traspaso original es del 2026-10-29). Kellu se
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

El encabezado de todos los `.md` apunta a `/assets/branding/svg/kellu-logo-color-*.svg`; esos
activos ya están en `main` (y en GitHub), así que el logo se ve.

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
  proveedor. Prueba añadida en M2 (`router_http::http_402_falls_back_to_the_next_provider`).
- La bitácora de sesiones en `docs/bitacora/` (hábito útil, se mantiene).
- Backlog de la sesión 03: opción `--provider` en la CLI, proveedor por defecto con saldo activo y
  alinear los identificadores de modelo (Gemini `gemini-flash-latest`, Gemma `gemma-4-26b-a4b-it`,
  OpenRouter `openrouter/free` o consulta de `/models`).

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
| Workspace | `crates/`: `kellu-common`, `kellu-security`, `kellu-sandbox`, `kellu-providers`, `kellu-projects`, `kellu-tools`, `kellu-core`, `kellu-frontends`, más el binario `kellu`. Dirección: `frontends -> core -> tools -> sandbox -> security -> common`; `providers` y `projects` también bajo `core`. Un crate se crea cuando empieza su hito; un test sobre `cargo metadata` lo exige. Ver la arista `sandbox -> security` en la sección 9.5. |
| Configuración | TOML en `~/.config/kellu/config.toml` sobre `defaults.toml` embebido (`include_str!`). Sin secretos, https obligatorio (http solo localhost), claves desconocidas son error, archivo propio y sin escritura de grupo u otros, máximo 256 KiB. |
| Keyring | Atributo `kellu=<servicio>_<clave>` y etiqueta `KELLU: <nombre>` (mismo formato que el prototipo, que usaba el atributo `kai`). Niveles deducidos del nombre: `ai_` es `provider`; `github_`, `cloudflare_`, `render_`, `desec_`, `duckdns_` son `infra`; `neon_`, `supabase_`, `mysql_` son `db`; desconocido cae en `infra`. `keyring_name = "x"` resuelve a `ai_x_api_key`. **Decidido en M3:** alta, lectura y baja con `secret-tool` (sin dependencia nueva, compatible con las entradas existentes); **listado por D-Bus con `busctl`** (`SearchItems` con filtro vacío y la propiedad `Attributes` de cada ítem), que nunca pide un valor. `secret-tool search` no sirve para listar (ver sección 9.3). Sin `busctl`, `kellu keys list` y `cleanup-legacy` fallan con un mensaje claro y todo lo demás funciona. |
| Proveedores por defecto | openrouter, deepseek (compatibles con OpenAI); google (`gemini-flash-latest`, alias móvil); gemma_direct (`gemma-4-26b-a4b-it`, clave y cuota propias); grok (`grok-4.3`, base `https://api.x.ai/v1`); groq (`llama-3.3-70b-versatile`, base `https://api.groq.com/openai/v1`, para probar el ingreso de claves). Google usa `v1beta`. Rutas: general = gemma_direct, groq, openrouter (provisional, para pruebas); code = deepseek, openrouter; explain = google, openrouter. Presupuesto total del router: 120 s. |
| HTTP | **`ureq` 3.5 con `rustls`** (decidido y verificado en M2): cero redirecciones (`max_redirects(0)` devuelve el 3xx sin seguirlo), timeouts por operación y global, streaming incremental, tope de cuerpo también con `chunked`, y recibe `http::Request`, así que la URL la parsea nuestro `http::Uri`. Proveedor criptográfico `ring` (no es Rust puro: C y ensamblador). Raíces de certificados: **Mozilla incluidas** (`webpki-roots`), no el almacén del sistema: una CA privada o un proxy que intercepta TLS no son de confianza (revisar con `platform-verifier` si hace falta). Los endpoints https respetan `ALL_PROXY`/`HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` del entorno (el proxy solo ve un túnel `CONNECT`); los endpoints `http` (solo localhost) **nunca** usan proxy: `ureq` no exime a localhost por sí mismo y un proxy vería la clave en claro (hallazgo de la revisión independiente; prueba `proxy::plain_http_ignores_the_proxy_environment`). |
| Sandbox | bubblewrap con `argv` como lista, sin shell. Política de versión **B**: se permiten versiones anteriores a 0.12.0 con advertencia (CVE-2026-87766), porque el diseño nunca crea puntos de montaje dentro del árbol del proyecto; modo estricto opcional. Ubuntu 24.04 trae 0.9.0 y su paquete retiró el parche en una actualización. Landlock (crate `landlock`) como respaldo posterior. **Decidido en M4:** siempre una **copia de trabajo** de los archivos del `Grant` montada en `/work` (nunca el proyecto real, ni en solo lectura); `--unshare-user` explícito (sin él un bwrap setuid podría correr sin namespace de usuario) más `--disable-userns`; límites con `prlimit` **dentro** del sandbox, alrededor del comando (ver sección 9.2); sin límites disponibles el comando se rechaza; versión ilegible o aislamiento que falla: no abre. |
| Autorización | Lectura por proyecto (solo lectura). Escritura nunca permanente: el modelo propone diffs con hashes, el usuario elige denegar, parcial o completo, y **solo el núcleo aplica**. Ningún sandbox escribe en el proyecto real; las herramientas que escriben trabajan sobre una copia (staging) y su resultado vuelve como otra propuesta. Un solo `Grant` deriva la política de rutas y los montajes (prueba con `proptest`). Auditoría JSONL append-only (0600) con cadena de hashes. Acceso a archivos por descriptores (`cap-std` o `rustix openat2`). |
| LSP en Kate | 0.1.0: hover y completion manual. 0.2.0: diagnósticos y code actions (una acción por hunk más "aplicar todo"). Chat, vista previa de diffs y texto fantasma necesitan otros frontends. Raíz igual a `$HOME`, sus ancestros o `/` se rechaza. La configuración de `.kateproject` es no confiable y solo puede endurecer. Candidato: `lsp-server` y `lsp-types`. |
| Marca | Azul marino `#14253D`, dorado `#C8A66A`, cobre `#B87355`, marfil `#F4F0E7`. Símbolo: geometría abstracta basada en figuras de textiles mapuches. Pendiente: versión simplificada para 16 y 24 px, rediseño vectorial propio, tipografía libre modificada por el usuario con registro de licencia (`docs/brand/PROVENANCE.md`). Dorado sobre marfil no alcanza contraste (2.0:1): usar un dorado más oscuro en fondos claros. Terminal: niveles ASCII `<*>`, Unicode `‹ ◆ ›`, color; respetar `NO_COLOR`, `TERM=dumb` y locale no UTF-8 (`kellu-frontends`, módulo `theme`, M6). |
| Encabezado de los `.md` | Todos los `.md` llevan el mismo encabezado con el logo (`/assets/branding/svg/kellu-logo-color-{dark,light}.svg`) y el lema "Local-first, modular AI assistant & LSP CLI built with Rust" (decidido: sin "Ruby &"). |

## 3. Hitos
| Hito | Prototipo Python (`original-python`) | Rust (`main`) |
|---|---|---|
| M0 | Hecho | **Hecho.** Workspace, toolchain fijada, `deny.toml`, CI, `kellu --version`, `doctor`, rutas XDG, archivos privados, errores, test de arquitectura |
| M1 | Hecho | **Hecho (paridad).** Config TOML estricta, logging privado con gancho de redacción, sesiones, `config path|check|show`, comprobación de config en `doctor` |
| M2 | Hecho | **Hecho (paridad).** Proveedores (OpenAI-compatible con streaming y herramientas, Google), transporte HTTP endurecido, router con respaldo |
| M3 | Hecho | **Hecho** (sus bloques de seguridad los consume M5, ver sección 9.1). Bóveda de keyring con niveles, `kellu keys list|set|rm|cleanup-legacy`, entorno depurado, deny globs, redactor v1, permisos por riesgo, scope y `Grant`, auditoría, `kellu providers test` |
| M4 | Hecho | **Hecho** (sus bloques los consume M5, ver sección 9.1). Crate `kellu-sandbox`: política derivada del `Grant`, copia de trabajo, bubblewrap con `argv`, límites de recursos con `prlimit`, tiempo máximo y tope de salida; sondeo de bubblewrap, `prlimit` y `openat2` en `doctor`; `[sandbox] require_patched_bwrap`; tests de escape contra bubblewrap real |
| M5 | No empezado | **Hecho.** Registro de herramientas con riesgo obligatorio, propuesta y aplicación de diffs, archivos, shell en sandbox, git de lectura; escrituras de git y gh en el host con confirmación (ver sección 9) |
| M6 | No empezado | **Hecho.** REPL (`kellu chat`), LSP (hover y completion, opt-in) y tema de terminal; ver sección 9 |
| M7 | No empezado | Documentación y etiqueta 0.1.0 (rama `m7-release`; la etiqueta se crea cuando el usuario lo confirme) |
El prototipo Ruby avanzó en paralelo en keyring (altas, bajas, lectura y listado), configuración y
adaptadores, así que es una segunda referencia para M2 y M3.

### 3.1 Criterios de diseño y plan después de 0.1.0 (dichos por el usuario, 2026-11-13)

1. **Ligero en memoria:** sin JVM residente ni procesos cargados sin necesidad.
2. **Sin atarse a bibliotecas de KDE**, salvo las que ya vienen en el paquete de Kate. LSP y CLI son
   el acoplamiento más débil con Kate (solo entrada y salida estándar); un plugin de KDE, el más fuerte.
3. **Soluciones propias en Rust si hacen falta;** en C solo tras analizarlo y como proceso aparte
   dentro del sandbox, nunca enlazado al workspace (`#![forbid(unsafe_code)]`).
4. **Corrector gramatical para Markdown (0.2.0 o después).** El usuario escribe en español, inglés y
   alemán, y algunos proyectos en francés, italiano, hebreo, griego y latín. Hechos verificados el
   2026-11-13: LanguageTool 6.6 cubre es, en, de, fr (reglas abundantes), it y el (pocas reglas, poca
   actividad) y **no** he ni la; LTeX+ (`ltex-ls-plus`) es un servidor LSP con Markdown que trae su
   propio Java y puede apuntar a un servidor remoto (`languageToolHttpServerUri`); la API pública
   permite 20 peticiones por minuto y 20 KB por petición y **prohíbe el uso automático** (hay que usar
   una instancia propia o Enterprise); Harper (`harper-ls`, Rust) solo documenta inglés; `nlprule`
   (Rust) cubre en, de y es (experimental) con reglas de LanguageTool 5.2 y sin mantenimiento
   confirmado. Plan: (a) en 0.2.0 probar que LanguageTool funciona con Kate y **medir su memoria**
   (RSS) con un `.md` normal; (b) es/en/de/fr con LanguageTool (servidor propio o nube premium, vía
   HTTPS sin Java local); it/el con el modelo como segunda opinión; he/la con el corrector de Kate y
   una acción de código del modelo; (c) solución propia en Rust solo si hace falta y salvo que
   LanguageTool publique una versión ligera sin Java (comprobarlo entonces; hoy no existe).
5. **Decisiones abiertas de ese trabajo:** dónde guardar la clave premium (`keyring_name` solo admite
   `ai_*`: haría falta otro nivel); qué se envía (deny globs y redacción que **conserve las
   posiciones**); cuándo se llama (nunca al teclear con la API pública); el módulo de **regiones
   Markdown** (vallas con su lenguaje, front matter, código en línea, mermaid) y cómo decidir el
   idioma por archivo, sección o bloque; si Kate admite varios servidores por documento (el usuario
   lo probará; condición previa para convivir con LTeX+).

Después: 0.2.0 agente (loop con herramientas, diagnósticos y code actions); 0.3.0 gateway
compatible con OpenAI y ACP; 0.4.0 memoria (SQLite, pgvector opcional); 0.5.0 runtimes locales,
perfiles de hardware y MCP.

## 4. Inventario de claves (solo nombres)
- provider: `ai_openrouter_api_key`, `ai_google_api_key`, `ai_deepseek_api_key`, `ai_grok_api_key`, `ai_groq_api_key`, `ai_gemma_direct_api_key` (ya copiada; borrar la entrada antigua `gemma_direct_api_key` tras verificar).
- infra: `github_token`, `cloudflare_tunnel_token`, `cloudflare_workers_token`, `cloudflare_r2_token`, `cloudflare_pages_token`, `cloudflare_api_token`, `render_api_key`, `desec_token`.
- db: `neon_admin_key`, `supabase_admin_key`.
- Borrar: `test_key`. No migradas (probablemente nunca guardadas): `cloudflare_zone_token`, `duckdns_token`, `mysql_*`.
- Siguen existiendo las entradas del prototipo bajo el atributo `kai`; `kellu keys cleanup-legacy` (en seco) las lista y `--apply` borra **solo** las que ya existen como entradas `kellu`; las demás se conservan.
- Abiertas: revocar o reducir `cloudflare_api_token` (token completo, ya hay acotados) y rotar el PAT de GitHub por uno fine-grained sin `delete_repo`.
- `secret-tool search` imprime los secretos: filtrar con `grep -E '^(label|attribute\.)'`. Nunca pegar valores en el chat.

## 5. Pendientes del usuario
- Crear las ramas (comandos arriba). Considerar reservar `kellu` en crates.io.
- Rellenar `<copyright holder>` (LICENSE y `authors` en `Cargo.toml`), `<your-user>` (READMEs y `repository` en `Cargo.toml`) y el contacto de seguridad de `SECURITY.md`.
- Fijar las acciones de GitHub por SHA antes de publicar (el CI de Rust ya existe; falta ejecutarlo en GitHub por primera vez).
- Aplicar el encabezado a los `.md` de `docs/branding/` (no pasaron por la sesión): `add-md-header.sh`, idempotente.
- Borrar `test_key` y la entrada antigua de gemma_direct.
- Confirmar en su cuenta el identificador de modelo de Grok y que los alias de Gemini respondan.
- Ejecutar `kellu doctor` y, como usuario normal, `KELLU_REQUIRE_SANDBOX_TESTS=1 cargo test -p kellu-sandbox --test escape`; contar lo que diga el sondeo.
- Probar M6 en su equipo (ver el cierre en la sección 9): `kellu chat` en una terminal y `kellu lsp` en Kate con `[lsp]` activado.
- Probar en su Kate: si las code actions aparecen sin diagnóstico, `showMessage` y `$/progress`, y si `.kateproject` puede sobrescribir el comando del servidor.
- Marca: versión de 16 px, vectorización, fuente libre y su registro de licencia.
- Mapas de bits en `assets/branding/png/` (varios tamaños, para quien no tenga soporte SVG): se hacen a mano para ajustar el espaciado del isotipo y del wordmark cuando se reducen mucho. Sin prioridad.
- `packaging/kellu.desktop`: ya está en el repo (`Exec=konsole -e kellu`, `Icon=utilities-terminal`). Desde M6 `kellu` sin argumentos abre el chat cuando hay terminal (Konsole lo es), así que la ventana ya no se cierra; cambiar el icono por el de la marca cuando existan los PNG.

## 6. Cómo trabajar
- Entorno de verificación (se repite en cada sesión; no persiste):
  `apt-get install -y cargo-1.91 rustc-1.91 rust-1.91-clippy rustfmt-1.91 bubblewrap` y
  `export PATH=/usr/lib/rust-1.91/bin:$PATH`. Para la prueba contra un keyring real:
  `apt-get install -y gnome-keyring libsecret-tools dbus dbus-x11` y
  `kellu/tests/real-keyring.sh target/debug/kellu` (sesión D-Bus y llavero desechables).
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
- Llamadas reales a las APIs de los proveedores (ninguna se hizo: no hay claves ni red hacia ellos
  en el entorno de verificación) y los identificadores de modelo de `defaults.toml`.
- Handshake TLS contra servidores reales y las raíces `webpki-roots` frente a los certificados
  reales de OpenRouter, DeepSeek, Google y xAI. Lo que sí se comprobó a mano: un servidor
  `openssl s_server` con certificado autofirmado es rechazado, y `ureq` lo entrega como
  `io::Error` de tipo `InvalidData` con el error de `rustls` dentro; el transporte lo clasifica
  como error TLS no reintentable (clasificación probada con una prueba unitaria, no con un
  servidor TLS en los tests, que dependería de `openssl`).
- Tiempo de compilación y tamaño del binario con `ring` y `ureq` (no medidos); `ring` en la
  arquitectura y el compilador del equipo del usuario.
- Comportamiento de `ureq` con proxies reales del entorno del usuario en https (solo se probó que
  los endpoints `http` los ignoran, con un proxy muerto).
- Que `secrecy` borre todas las copias de una clave: las que `ureq` y `http` hacen al armar la
  cabecera no se borran.
- **KWallet / ksecretd (KDE).** Todo lo del keyring se verificó contra **gnome-keyring**. El listado
  por D-Bus (`SearchItems` vacío y `Attributes`) y `secret-tool` contra la implementación de
  Secret Service de KWallet no se probaron; es lo primero que hay que probar en un Plasma.
- `openat2` (Linux 5.6 o más) en el kernel y el seccomp del equipo del usuario: sin él `Scope`
  rechaza leer archivos (falla cerrado); el camino de `ENOSYS` solo se probó por lectura de código.
- Calidad real del redactor v1: no se midió contra un corpus de secretos y código reales (falsos
  negativos y positivos de la heurística de entropía). Es de mejor esfuerzo, igual que en `SECURITY.md`.
- Copias residuales de un secreto en memoria: el búfer interno de `std` al leer stdin en
  `keys set` y lo que haga `secret-tool` con su proceso no se borran (`secrecy`/`zeroize` solo
  cubren nuestras copias).
- La auditoría con **varios procesos** reales escribiendo a la vez (se probó con 8 hilos y `flock`).
- **Sandbox (M4) en el equipo del usuario.** Todo se probó con bubblewrap **0.9.0**, como root y como
  un usuario sin privilegios, en un contenedor. No se probó con 0.12 o más, con bubblewrap de Flatpak
  o Snap, ni en una distribución que restrinja los namespaces de usuario sin privilegios (Ubuntu 24.04
  puede hacerlo con AppArmor): `kellu doctor` lo informa, no lo resuelve.
- **Red dentro del sandbox.** Con red permitida solo se probó el `argv` (`--share-net`). La resolución
  de nombres es dudosa: `/etc/resolv.conf` suele ser un enlace a `/run`, que no se monta.
- **El aviso.** Que 0.12.0 corrige GHSA-pxhw-h44j-8pfx está en las notas de la versión (leídas). Que
  ese GHSA es el CVE-2026-87766 sale de resultados de búsqueda; el aviso de GitHub no se pudo leer.
- **Memoria virtual de runtimes** (Java, Go, Node) con el límite por defecto de 4 GiB.
- **M6 (frontends).** Ver la sección 9: nada se probó con un Kate real ni con un proveedor real, y el
  REPL no se probó en una terminal interactiva.
- **M5 (herramientas).** `gh` real nunca se ejecutó: solo un script falso que imita su `argv` y su
  stdin. `git_push` se probó solo contra un remoto bare local (sin SSH ni agente, sin `gh`). Git se
  probó únicamente con la versión del contenedor. El `Approver` y las confirmaciones no se probaron
  con personas ni con un frontend real. La prueba de "archivo de otro usuario" solo corre como root.
  El guardián de config es una lista negra: otras claves de git que ejecuten programas no están
  cubiertas hasta que se descubran.
- Los comportamientos de Kate señalados en la sección 5; el CI en
  GitHub (no se ha ejecutado); las pruebas de "archivo o directorio de otro usuario" solo corren
  cuando los tests se ejecutan como root.

## 8. Archivos de este traspaso
`HANDOFF.md`, `README.md`, `README.es.md`, `SECURITY.md`, `CHANGELOG.md`, `LICENSE`,
`docs/ARCHITECTURE.md`, `docs/PORTING.md`, `docs/porting/python-tests.txt`, `docs/porting/PARITY.md`,
`docs/design/permissions-and-sandbox.md`, `docs/design/lsp-capabilities.md`, `packaging/kellu.desktop` y el workspace de Cargo completo (`kellu-M1.zip`). `kellu-project-instructions.md` vive como
instrucciones del proyecto y no está en el repositorio. El prototipo de Python completo
(`kellu-M2.zip`) es para la rama `original-python`.

## 9. Estado al cierre de M6 (2026-11-09)

**Verificado** (Rust 1.91.1 por apt, bubblewrap 0.9.0, git del contenedor, kernel 6.18):
`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` y
`cargo test --workspace --locked` con `KELLU_REQUIRE_SANDBOX_TESTS=1`: 576 tests, 0 fallos (eran 510
al cierre de M5: +11 en `kellu-core` entre `assist` y `[lsp]`, +49 en `kellu-frontends`, +6 en el
binario). `lsp` (21), `repl` (8) y el binario (28) también pasan como `nobody`. `cargo deny check` y
`cargo audit` (89 paquetes, 1422 avisos) en verde. Mutaciones comprobadas en el servidor LSP: sin la
comprobación de deny, sin la de symlink, sin resolver el directorio, con `initializationOptions` que
pueden activar, sin repetir la comprobación en cada petición y con caracteres de activación hacen
fallar tests (dos mutaciones sobrevivieron al principio y se añadieron los tests que faltaban).
Prueba de humo real: `kellu lsp` contra un servidor HTTP local que imita la API OpenAI, con `[lsp]`
activado: hover y completion responden, un `.env` abierto no llama al proveedor y ni el token de
ejemplo ni el contenido de `.env` aparecen en lo enviado.

**Hecho en M6** (sin dependencias externas nuevas; `Cargo.lock` solo añade `kellu-frontends`):
- `kellu-frontends` (nuevo): `theme` (ASCII `<*>`, Unicode `‹ ◆ ›`, color; baja por `NO_COLOR`,
  `TERM=dumb` o sin definir, locale no UTF-8 o salida que no es terminal), `repl` y `lsp`
  (`transport`, `uri`, `text`, `server`).
- `kellu-core::assist`: único punto por el que el texto del editor se vuelve petición al modelo:
  ventana acotada, redacción, valla `<kellu-code>` con los marcadores del texto rotos, id de lenguaje
  reducido a ASCII, respuesta sin caracteres de control ni vallas de Markdown, historial del REPL
  acotado y redactado al salir.
- Configuración `[lsp]` (`hover`, `completion`, ambos `false`).
- CLI: `kellu chat` (y `kellu` sin argumentos en una terminal; si no es terminal, imprime la ayuda) y
  `kellu lsp`. Ya no queda ningún comando "planned": se eliminó `EXIT_NOT_IMPLEMENTED`.
  Ambos arrancan sin keyring (los proveedores que necesitan clave fallan con error claro).

**Decisiones tomadas en M6 (con motivo):**
1. **Hover y completion vienen desactivados.** Kate los pide sola (pausa del ratón, identificador
   tecleado) y el protocolo no distingue la petición manual; cada una envía código a un proveedor. Un
   proyecto (`.kateproject`, no confiable) solo puede apagarlas. **A confirmar contigo.**
2. LSP escrito a mano sobre `serde_json` (sin `lsp-server` ni `lsp-types`, como proponía PORTING):
   ocho mensajes no justifican el árbol de dependencias, y así el encuadre es estricto y se prueba.
3. Documentos: solo URIs `file:`; deben caer dentro del `Scope` (rechaza `/`, `$HOME`, sus ancestros
   y raíces sin marcador de proyecto), pasar los deny globs y no ser ni estar tras un symlink; se
   resuelve primero el directorio y se repite la comprobación en cada petición. Un búfer sin guardar
   se admite si su directorio es válido. Una raíz rechazada no rompe el handshake: apaga las funciones.
4. Sincronización de texto completa; sin caracteres de activación; hover en texto plano; completion
   como una sola edición de texto plano (`insertTextFormat` 1: un `$1` del modelo nunca se expande).
5. Errores del modelo ⇒ resultado vacío y una línea en el log privado, nunca un mensaje a Kate.
6. El REPL no usa herramientas: el bucle de agente es 0.2.0. Comandos: `/help`, `/clear`, `/exit`.
7. `kellu chat`/`lsp` no exigen keyring: avisan por stderr y siguen.

**Abiertos tras M6:**
1. El servidor es de un solo hilo: una llamada al modelo bloquea a Kate hasta que responde o vence el
   presupuesto del router. `$/cancelRequest` se ignora. Sin tope de llamadas por minuto (solo la
   caché por versión de documento).
2. Patrones deny del usuario siguen sin leerse de `config.toml` (pendiente desde M3); solo la lista
   incorporada.
3. `kellu-projects` sigue sin crearse (decidir con 0.2.0).
4. Sin aviso visible en Kate cuando falla el modelo (`window/showMessage` es 0.2.0, comportamiento
   por verificar).
5. El REPL no muestra aún propuestas ni pide confirmaciones (llegan con el bucle de agente).
6. Un `Approver` y las confirmaciones de M5 siguen sin probarse con personas.

**Probado después por el usuario en un Kate real (2026-11-13):** hover y completado funcionan con
archivos `.rs` dentro de un repo con `.git`. Un archivo Python no activó Kellu porque la entrada era
solo para Rust (Kate arrancó `pylsp`); la clave `python` del ejemplo anterior del README habría
reemplazado `pylsp`, por eso el ejemplo ahora usa la clave `kellu`.

**Corrección posterior (2026-11-13):** hover dejó de aparecer porque Kate abrió Kellu con la raíz
`/home/claudio` (su respaldo cuando no reconoce un proyecto) y `Scope` la rechaza; el log privado lo
decía. Ahora, si la raíz del cliente se rechaza, cada documento usa su propio proyecto (el directorio
más cercano con marcador, nunca el home ni sus ancestros; máximo 8) y una raíz válida nunca se
amplía. El proceso `kellu lsp` ocupó unos 4 MB (RSS) en el equipo del usuario.

**Diagnóstico (2026-11-13, rama `m7-lsp-diagnostics`):** tras ese arreglo el hover seguía sin aparecer
y el log no mostraba causa. Ahora hover/completion registran una vez por motivo por qué no devuelven
nada (documento rechazado, respuesta vacía del modelo, fallo de la petición), sin texto del documento.
Pendiente del usuario: reinstalar, reintentar y enviar las líneas `lsp:` del log.

**No verificado:** con un Kate real, los demás modos de la lista (Markdown, YAML, etc.), varios
servidores activos por documento (el usuario lo probará), con qué frecuencia pide Kate hover y
completion y si `.kateproject` puede sobrescribir el comando. Con un proveedor real tampoco: solo un servidor
HTTP local falso. El REPL no se probó en una terminal interactiva real (tema de color, Ctrl-C,
Ctrl-D, pegado largo); sí con flujos en memoria y como proceso con tuberías. La prueba de "otro
usuario" de M5 sigue corriendo solo como root.

**Siguiente: M7** (documentación y etiqueta 0.1.0). Antes, en tu equipo: `cargo test --workspace`,
`kellu chat` en una terminal y, en Kate, el JSON de `servers` del README con `[lsp]` activado
(primero `completion`, luego `hover`), anotando qué se ve y con qué frecuencia llama.

## 9.1 Estado al cierre de M5 (2026-11-08)

**Verificado** (Rust 1.91.1 por apt, bubblewrap 0.9.0, git del contenedor, kernel 6.18):
`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` y
`cargo test --workspace --locked` con `KELLU_REQUIRE_SANDBOX_TESTS=1` (510 tests, 0 fallos; eran 434 al
cierre de M4: +66 en `kellu-tools`, +6 del ejecutor de host, +3 de `kellu-security`, +1 de CLI). Los
binarios `proposal_apply` (21), `registry` (13), `git` (11) y `shell` (10) se repitieron **como
`nobody`**. Mutaciones comprobadas: sin el guardián de git fallan los tests de filtros y de config
peligrosa; sin `Grant::without_network` falla el test de red del shell. `cargo deny check`: advisories,
bans, licenses y sources en verde (el aviso de la licencia `Zlib` sin usar ya existía). `cargo audit`:
88 paquetes en `Cargo.lock`, 1422 avisos, sin hallazgos.

**Hecho en M5** (sin dependencias externas nuevas; `Cargo.lock` solo añade `kellu-tools` y su arista):
- `kellu-tools` (nuevo): `spec` (`ToolSpec` con `Risk` obligatorio, `Context`, `Tool`), `args` (argumentos
  estrictos), `proposal` (`Item`/`Hunk`/`Proposal`), `apply` (`approve` y `apply`), `fs_tools`
  (`list_files`, `read_file`, `search_text`, `propose_edit`, `propose_create`, `propose_delete`), `shell`
  (`run_command`), `git` (`git_status`, `git_diff`, `git_log`; `git_commit`, `git_push`,
  `gh_pr_create`), `host` (ejecución en el host) y `registry` (puerta por riesgo).
- `kellu-security`: `Scope::open_dir` y `Grant::without_network`.
- `kellu-sandbox`: `HostCommand` y `run_host`, ejecutor de host compartido (entorno explícito, stdin,
  grupo de procesos, tiempo máximo y tope de salida).
- CLI: `kellu tools` lista cada herramienta con su riesgo y no crea nada. `kellu doctor` comprueba `gh`
  como opcional. Solo `lsp` sigue como "planned" (M6).

**Hallazgos:**
1. `filter.*.clean` de git **ejecuta programas** aunque se endurezca con `-c` (medido). El guardián de
   `.git/config` rechaza `filter.*.{clean,smudge,process}`, `include`/`includeIf`, `core.sshCommand`,
   `core.askPass` y `gpg.*.program`.
2. Un test que "pasa" como root puede saltarse la comprobación real (permisos): el rollback se cubre con un
   test unitario (directorio borrado tras abrir el fd) y se ejecuta también como `nobody`.
3. El shell sin red debía derivarse del grant, no de un parámetro: `Grant::without_network`.

**Decisiones tomadas en M5 (con motivo):**
1. La puerta por riesgo corre **antes** de `tool.run`; si falla la auditoría, la herramienta no corre. La
   salida pasa por `Redactor`, `strip_control_chars` y un tope de 60 000 caracteres.
2. Las `propose_*` son `Read`: no tienen efectos; solo `apply` escribe.
3. Los hunks se resuelven a offsets de bytes contra el SHA-256 del archivo visto; la aprobación es por
   ítem o por hunk. `apply` trabaja con fd relativo a `Scope`, `NOFOLLOW`, recomprueba el hash, escribe a
   temporal + `rename`, deja backup 0600 y revierte si algo falla. Rechaza archivos de otro usuario o con
   enlaces duros; no crea directorios; no copia ACL ni xattr.
4. Borrar es `Destructive`: el usuario teclea la ruta exacta.
5. `run_command` corre en bubblewrap sobre una copia y **siempre sin red**; los cambios vuelven como
   propuesta (`write_back`).
6. Git y `gh` en el host con entorno vacío, `-c core.fsmonitor=false -c core.pager=cat
   -c core.hooksPath=/dev/null`, `--no-ext-diff --no-textconv --ignore-submodules=all` y filtrado por la
   deny list (primero nombres; el diff solo de los permitidos).
7. Escrituras con forma fija: `git_commit` (hooks desactivados, mensaje por stdin), `git_push`
   (`HEAD:refs/heads/<rama>` a `origin`, sin force) y `gh_pr_create` (`--title=`, `--body-file=-`). Sin
   amend, sin borrar ramas. Entorno con lista de permitidos (HOME, XDG_CONFIG_HOME, XDG_RUNTIME_DIR,
   SSH_AUTH_SOCK, LANG, LC_ALL, LC_CTYPE, TERM) y PATH fijo; **sin `GH_TOKEN`**.

**Abiertos tras M5:**
1. Nada llama aún al registro: lo consume M6 (REPL y LSP). `Approver` y `Confirmation` no se han probado
   con humanos.
2. Los límites no son configurables por clase de herramienta (M4 lo anticipaba); siguen los valores
   provisionales de la sección 9.2.
3. `kellu-projects` no se creó: `ARCHITECTURE.md` lo ligaba a M5, la tabla de hitos no. Decidir en M6.
4. El guardián de config es una **lista negra**: lo que no esté en ella pasa.
5. `git_commit` desactiva los hooks (`core.hooksPath=/dev/null`): **a confirmar contigo**; el costo es que
   los hooks de pre-commit del proyecto no corren.
6. TOCTOU residual en `apply` (sin lock entre procesos). Cada `propose_*` trata un archivo por llamada.
7. Sin herramienta de red dentro del sandbox. `.venv` y seccomp siguen como en la sección 9.2.

**Siguiente: M6** (REPL, LSP con hover y completion, tema de terminal). Antes, en tu equipo y como usuario
normal: `kellu doctor`, `kellu tools` y `KELLU_REQUIRE_SANDBOX_TESTS=1 cargo test -p kellu-tools --test shell`
y `--test git`.

## 9.2 Estado al cierre de M4 (2026-11-05)

**Verificado** (Rust 1.91.1 por apt, bubblewrap 0.9.0 del paquete de Ubuntu 24.04, kernel 6.18):
`cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` y
`cargo test --workspace --locked` con `KELLU_REQUIRE_SANDBOX_TESTS=1` (434 tests, 0 fallos; eran 368
al cierre de M3). La suite de escape (20 tests contra bubblewrap real) se repitió 5 veces **como
root** y 5 veces **como usuario sin privilegios**, sin intermitencias. Además `cargo deny check` (advisories, bans, licenses y sources en verde; el aviso de que la licencia `Zlib` no se usa ya existía) y `cargo audit` (87 paquetes en `Cargo.lock`, 1409 avisos, sin hallazgos).

**Hecho en M4** (sin dependencias externas nuevas; `Cargo.lock` solo añade el crate):
- `kellu-sandbox`: `policy` (la política sale de **un** `Grant`: sus archivos legibles, su red, el perfil
  `read-only` o `workspace-write`, límites y entorno depurado; caduca con el grant), `staging`
  (copia por `Scope`/`openat2`, directorios 0700 y archivos 0600 con solo el bit de ejecución,
  topes de archivos y bytes, se borra al soltarla; `changes()` informa modificados, creados,
  borrados e inseguros sin seguir enlaces ni aplicar nada), `bwrap` (`argv` como lista tras un `--`,
  raíz tmpfs, sistema de solo lectura, `/home` y `/tmp` vacíos, `--unshare-all --unshare-user
  --disable-userns --die-with-parent --new-session --cap-drop ALL --clearenv`, red solo si el grant
  la permite), `limits` y `run` (`prlimit`, tiempo máximo con muerte del grupo de procesos, tope de salida) y
  `probe`.
- `kellu doctor`: comprueba que bubblewrap puede crear el sandbox con las **mismas opciones** que usa
  Kellu, su versión frente a 0.12.0, que `prlimit` funcione dentro del sandbox y `openat2`.
- `config.toml`: `[sandbox] require_patched_bwrap` (por defecto `false`).
- `kellu_security::openat2_available()`.
- Tests: 34 unitarios y 20 de escape. El "test que compara política y montajes" de M3 son tres
  piezas: la política es igual a lo legible del grant, el `argv` monta solo la copia en `/work`
  (el resto es sistema de solo lectura) y la copia es **exactamente** el grant (400 nombres hostiles
  generados con semilla fija; sin `proptest`, que sería una dependencia de desarrollo grande para un test).

**Hallazgo que cambió el diseño:** la primera versión envolvía `bwrap` con `prlimit --nproc=256`.
Pasaba todos los tests como root y como usuario con pocos procesos, pero `RLIMIT_NPROC` cuenta **todos
los procesos del usuario**: con 306 procesos y límite 256, `bwrap` falla con `Creating new namespace
failed: Resource temporarily unavailable`; un escritorio normal supera eso. Los límites se aplican ahora
**dentro** del sandbox, solo al comando: allí cuenta únicamente los procesos del sandbox (medido con
304 procesos fuera: 100 hijos con límite 256 funcionan, y con límite 20 el fork falla). Hay una prueba
de regresión (350 procesos ajenos) y el test del límite de procesos solo es significativo como
usuario normal (como root se omite).

**Decisiones tomadas en M4 (con motivo):**
1. Límites dentro del sandbox, ver arriba. Además no hay forma segura de usar `setrlimit` en el hijo
   (`pre_exec` exige `unsafe`, prohibido). Si `prlimit` falta o no es visible dentro del sandbox
   (debe estar bajo `/usr` o `/bin`), el comando **se rechaza**, no corre sin límites.
2. Una copia de trabajo siempre, también en solo lectura: así el proyecto real no se monta nunca y el
   invariante de montaje (hoja única en `/work`, sin montajes anidados) se cumple por construcción.
   Enlaces simbólicos, archivos especiales y archivos grandes se **omiten y se informan** (`skipped`);
   superar el tope de archivos o de bytes totales **aborta**, porque una copia parcial engañaría.
   Sin `reflink`: se copia leyendo por `openat2`, que es lo que da la garantía sin enlaces.
3. `bwrap` informa una muerte por señal como código `128 + señal`, así que `Outcome::Exited(137)`
   y una señal real no se distinguen; `Outcome::signal()` aplica la convención del shell.
4. Política de versión B por defecto: bubblewrap anterior a 0.12.0 se acepta, y `kellu doctor` avisa
   con el aviso (GHSA-pxhw-h44j-8pfx / CVE-2026-87766) y el interruptor; el modo estricto lo rechaza.
5. Un binario que no es un bubblewrap (versión ilegible) o que no aísla no abre el sandbox.

**Abiertos tras M4:**
1. Los valores por defecto de los límites son provisionales: CPU 60 s, memoria virtual 4 GiB (puede
   quedarse corta para Java, Go o Node), 256 procesos, archivo de 64 MiB, 1024 descriptores, reloj de
   120 s, salida de 1 MiB y copia de 20 000 archivos, 8 MiB por archivo y 256 MiB en total. M5 los
   hace configurables por clase de herramienta.
2. Los entornos virtuales del proyecto (`.venv`) **no** se montan: los enlaces no se copian, así que un
   venv típico no funciona dentro del sandbox. El perfil `unsafe` no existe todavía. Sin filtro
   seccomp: el aislamiento es de namespaces, capacidades y límites.
3. Siguen abiertos los de M3 (`keys migrate`, deny del usuario en `config.toml`) y M2.

**Qué aún no consume nada:** el sandbox está probado, pero ninguna herramienta lo usa hasta M5, y
`Staging::changes()` aún no alimenta propuestas.

**Siguiente: M5.** Registro de herramientas con riesgo obligatorio; luego propuesta y aplicación de
diffs (reutiliza `Staging::changes()` y sus hashes); archivos; shell en sandbox; git de lectura. Antes,
ejecutar `kellu doctor` y `KELLU_REQUIRE_SANDBOX_TESTS=1 cargo test -p kellu-sandbox --test escape` en
tu equipo, como usuario normal.

## 9.3 Estado al cierre de M3 (2026-11-02)

**Verificado** (Rust 1.91.1 por apt): `cargo fmt --check`, `cargo clippy --workspace --all-targets
--locked -- -D warnings`, `cargo test --workspace --locked` (368 tests, 0 fallos, repetida 5 veces sin intermitencias), `cargo deny
check` (advisories, bans, licenses y sources en verde) y `cargo audit` (86 paquetes en `Cargo.lock`, 1402 avisos, sin hallazgos). Además,
**contra un llavero real**: `kellu/tests/real-keyring.sh` ejecuta el binario `kellu` sobre un
gnome-keyring real en una sesión D-Bus desechable (31 comprobaciones: alta por stdin,
reemplazo, listado con niveles, `cleanup-legacy` en seco y aplicado, baja, entorno mínimo; en ningún
punto aparece un valor en la salida), y una prueba con pseudo-terminal real (eco apagado mientras
se escribe, el secreto no se refleja, el eco se restaura). **No verificado:** ver la sección 7
(sobre todo KWallet).

**Hallazgo que cambió el diseño:** la primera versión del listado usaba
`secret-tool search --all xdg:schema ...` y pasaba todos los tests con un `secret-tool` falso, pero
contra un llavero real devolvía **una entrada de cuatro**. Además `secret-tool search` exige un par
atributo/valor (no hay "listar todo") e **imprime los secretos**. Por eso el listado se rehízo sobre
D-Bus: `SearchItems` con filtro vacío devuelve todos los ítems y se lee solo `Attributes` de cada
uno; nunca se pide un valor. Los formatos JSON de `busctl` de los tests son capturas reales.

**Hecho en M3:**
- `kellu-security` (nuevo; depende solo de `kellu-common`): `tier` (niveles por prefijo, desconocido
  cae en `infra`, `keyring_name` siempre a `ai_<x>_api_key`), `store` (`SecretStore`: `secret-tool`
  más `busctl`; `MemoryStore` para tests), `vault` (`Vault`: la única lectura posible es
  `provider_key`; `infra` y `db` se guardan, listan y borran pero **no tienen ruta de lectura**),
  `env` (entorno depurado por lista blanca), `redact` (`PatternRedactor`), `deny` (`DenyGlobs`),
  `scope` (`Scope`), `grant` (`Grant`), `permission` (`Risk`, `requirement`), `audit` (`AuditLog`,
  `verify`) y `hash`.
- CLI: `kellu keys list|set|rm|cleanup-legacy [--apply]` y `kellu providers test [nombre]`;
  `doctor` comprueba `busctl`. El secreto de `keys set` se lee solo por stdin (sin eco en
  terminal); pasarlo como argumento es un error de uso y no se guarda.
- `kellu-core`: `SessionLog` redacta **por defecto** con `PatternRedactor` (antes era la identidad);
  `log::default_redactor()`; `config show` calcula la entrada con `provider_entry`.

**Decisiones tomadas en M3 (con motivo):**
- `secret-tool` para alta, lectura y baja; `busctl` solo para listar. Un crate de Secret Service
  habría traído unos 200 paquetes al árbol (medido con `cargo tree`; hoy son 60 en total) por una
  sola función. `busctl` es parte de systemd; `doctor` lo marca como opcional.
- Los niveles se deducen del nombre y no se guardan. `keyring_name` solo puede apuntar a `ai_*`
  (probado: una configuración manipulada con `keyring_name = "github_token"` resuelve a
  `ai_github_token_api_key`, nunca a `github_token`, y el almacén solo recibe nombres `ai_*`).
- Los nombres de ruta D-Bus que devuelve el llavero se validan antes de llegar a `argv` de `busctl`.
- Redactor v1: patrones de formatos conocidos, bloques de clave privada, asignaciones
  (`api_key = ...`, `Authorization: Bearer ...`), valores exactos de claves registradas y entropía.
  Marcador fijo `[REDACTED]`: **los marcadores reversibles de `SECURITY.md` siguen en diseño**.
  Escaneo lineal, sin pánico con cualquier entrada.
- `Scope`: lectura con `openat2` (`RESOLVE_BENEATH | NO_SYMLINKS | NO_MAGICLINKS`) sobre un
  descriptor del directorio raíz, así que no hay carrera entre comprobar y abrir; la raíz no puede
  ser `/`, el home ni un ancestro (ni siquiera confirmando); exige marcador de proyecto salvo
  confirmación; sin `openat2` falla cerrado. `rustix` en lugar de `cap-std`: una dependencia menos.
- `Grant`: solo lectura al abrir el proyecto; la escritura se aprueba por archivo con el hash base
  que vio el usuario; red apagada por defecto; caduca. Un permiso de red sin perfil `network` es
  `Denied`, no `Confirm`.
- Auditoría: JSONL con cadena SHA-256, archivo `0600`, `flock` exclusivo, no anexa a un log
  corrupto, `verify` detecta edición, borrado y reordenamiento de líneas. Es **evidencia** de
  manipulación, no protección: quien pueda reescribir todo el archivo puede reconstruir la cadena.
- Dependencias nuevas: `sha2` 0.11 (más `digest`, `block-buffer`, `crypto-common`, `hybrid-array`,
  `typenum` y `cpufeatures`, todas RustCrypto), `zeroize` (ya estaba en el árbol por `secrecy`) y las
  características `stdio` y `termios` de `rustix`. `Cargo.lock`: +7 paquetes externos.
- `kellu-core` pasa a depender de `kellu-security` (arista ya permitida).

**Abiertos (decidir):**
1. **`busctl` como requisito para listar.** Recomendación: mantenerlo (opcional, con mensaje claro).
   Alternativa: un crate de Secret Service, a costa de ~200 paquetes.
2. **`keys migrate`** (copiar `kai` a `kellu`) está en `SECURITY.md` pero **no se implementó**: solo
   existe `cleanup-legacy`. Copiar exigiría leer el valor antiguo; como ya migraste a mano, propongo
   quitar `migrate` del diseño salvo que lo quieras.
3. Los patrones deny **del usuario** (`DenyGlobs::with_user_patterns`) no están conectados a
   `config.toml` todavía, ni tampoco el cambio de niveles por el usuario que describe `SECURITY.md`.
4. Siguen abiertos los de M2 (mantener `ring` y las raíces incluidas; Google sin `finishReason`).

**Qué aún no consume nada:** `Grant`, los permisos, `Scope`, los deny globs y la auditoría están
probados pero ninguna herramienta los usa hasta M4 y M5. Ningún comando envía contexto de archivos
a un proveedor (solo `providers test`, con un prompt fijo). El binario todavía no llama a
`configure_logging`, así que el log de la aplicación no se produce hasta M6.

**Siguiente: M4.** Sandbox: política de montajes derivada del `Grant` (un test que compare ambas),
copia de trabajo, límites de recursos, sondeo de bubblewrap y de `openat2` en `doctor`, y tests de
escape. Antes, probar `kellu keys list` y `kellu providers test` en tu equipo (sobre todo si usas
KWallet).

## 9.4 Estado al cierre de M2 (2026-11-01)

**Verificado** (Rust 1.91.1 instalado por apt; rustup no alcanza `static.rust-lang.org` desde el
entorno de verificación): `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked
-- -D warnings`, `cargo test --workspace --locked` (237 tests, 0 fallos; también con un proxy muerto en el entorno), `cargo deny check`
(advisories, bans, licenses y sources en verde) y `cargo audit` (78 crates, sin hallazgos). La
paridad (137 de 137 funciones de prueba) se comprobó con un script: cada función de
`python-tests.txt` está en `PARITY.md` y cada test Rust citado existe. **No verificado:** ver la
sección 7.

**Hecho en M2:** `kellu-providers` (tipos, trait `Provider`, transporte HTTP, cliente
OpenAI-compatible con streaming y herramientas, cliente Google con Gemini y Gemma, registro),
`kellu-core::router` (`classify_task`, respaldo en `chat` y `stream`, presupuesto de tiempo con reloj
inyectable), `KelluError::AllProvidersFailed`, servidor de prueba con guion y tests de extremo a
extremo (incluida la prueba nueva de HTTP 402 con respaldo, en chat y en streaming).

**Decisiones tomadas en M2 (con motivo):**
- `ureq` 3.5 con la característica `rustls` (ver sección 2, fila HTTP). Árbol normal de
  dependencias: de 30 a 52 crates. `deny.toml` admite `CDLA-Permissive-2.0` por la lista de CA.
- `serde_json` como dependencia normal, sin `serde` derive: las cargas se arman con `json!` y se
  leen como `Value`; los errores nunca repiten contenido de la respuesta sin pasar por `scrub`.
- `secrecy` para las claves (sin `zeroize` directo); el proveedor recibe un `KeyLookup` inyectado
  y consulta la clave en cada petición, sin guardarla. La bóveda real es M3.
- `Message` es un enum (un resultado de herramienta siempre lleva `call_id`); los argumentos de
  una llamada son un `Map`, siempre un objeto JSON.
- `EventStream` es un iterador perezoso de `Result`: la petición sale en el primer `next()` y todo
  fallo, incluido un estado HTTP de error, es un ítem `Err` (igual que el generador del prototipo).
- Plegado de acentos con una tabla explícita más descarte de marcas combinantes, sin
  `unicode-normalization`. `classify_task` conserva la semántica de Python (por ejemplo, un
  prompt que empieza por `¿` no cuenta como prefijo de explicación).
- Servidor de prueba propio y mínimo (`TcpListener`) en lugar de `tiny_http` (sin
  mantenimiento desde 2022).
- Desviaciones de paridad documentadas en `PARITY.md`: tope global de 600 s también para las
  peticiones no `stream` (el prototipo solo lo ponía a los streams), claves solo con ASCII
  imprimible, raíces TLS incluidas en lugar del almacén del sistema.
- `kellu-core` depende de `kellu-providers` (ya permitido). Dev-dependencies nuevas en `kellu-core`:
  `secrecy` y `serde_json`.

**Revisión independiente (un revisor sin contexto leyó el transporte y el router):** corregido el
proxy en endpoints `http`, los ids de herramienta duplicados o inseguros, las etiquetas de
modelo y motivo de fin sin sanear y el registro del fallo tras texto entregado. **Abiertos, sin
corregir** (decidir):
- Google acepta como completo un stream que termina limpiamente sin `finishReason` (el cliente
  OpenAI-compatible sí lo rechaza). Es la semántica del prototipo; endurecerlo puede dar falsos
  errores si la API real omite el campo en algún caso, y no se pudo comprobar sin llamadas reales.
- El presupuesto total del router solo se revisa antes de empezar cada proveedor: un stream en
  curso puede durar hasta el tope de 600 s por intento (igual que el prototipo).
- Un servidor que repite la clave en el cuerpo de error más allá de los 4096 bytes leídos deja un
  prefijo de la clave sin reemplazar; el depurado es por coincidencia exacta y no toca claves de
  menos de 8 caracteres.
- Un SSE con solo `\r` como separador de línea falla cerrado ("stream line too long").

**Aún no se puede usar desde la CLI:** no existe la bóveda (M3), así que `kellu` no llama a ningún
proveedor; `kellu config show` ya lista proveedores y rutas.

**Siguiente: M3.** Orden sugerido: (1) decidir `secret-tool` frente a un crate de Secret Service
(sección 2, fila Keyring) probando contra el esquema real `kellu=<servicio>_<clave>`;
(2) `kellu-security`: bóveda con niveles y `keyring_name` restringido a `ai_*`, entorno depurado,
deny globs, redactor v1 (cablearlo al gancho de `log` y `session`), permisos por riesgo, scope y
`Grant`, auditoría; (3) `kellu keys list|set|rm|cleanup-legacy` leyendo secretos por stdin sin
eco; (4) `kellu providers test`, conectando la bóveda con `KeyLookup`. Antes de M3, decidir si se
mantiene `ring` y las raíces incluidas (sección 2, fila HTTP).

## 9.5 Estado al cierre de M1 (2026-10-31)

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

**Lema del encabezado:** resuelto; el lema es "built with Rust" (sin "Ruby &") en todos los `.md`.
