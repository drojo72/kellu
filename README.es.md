# Kellu

[English](README.md) · [Español](README.es.md)

**Asistente de IA seguro y modular para el editor Kate, que piensa antes de gastar tokens.**

> **Estado: pre-0.1.0.** Se reconstruye desde cero con diseño de seguridad primero. Existe un prototipo anterior que se porta módulo por módulo. Las APIs, comandos y configuración cambiarán. Aún no se puede instalar desde PyPI. Modelo de seguridad: ver [SECURITY.md](SECURITY.md).

---

## ¿Qué es Kellu?

Kellu lleva asistencia de IA a [Kate](https://kate-editor.org/) sin una pila gráfica pesada. No es solo un chat con un LLM. Un núcleo pequeño y sin interfaz decide **si** hace falta un modelo, redacta lo que sale de tu máquina, enruta cada tarea al proveedor adecuado con respaldo, ejecuta dentro de un sandbox cualquier herramienta que lance procesos, y se comunica con el editor mediante frontends delgados.

## Principios

- **Seguridad primero, fallar cerrado.** Si una protección no está disponible (por ejemplo, no hay backend de sandbox), la capacidad peligrosa se desactiva en lugar de ejecutarse sin protección.
- **Mínimo privilegio.** Las claves viven en el keyring del sistema, separadas en niveles. El modelo nunca ve secretos. Las herramientas corren con el acceso mínimo necesario.
- **No confiable por defecto.** Archivos, páginas web y snippets guardados son datos, nunca instrucciones.
- **Cascada de decisión.** Reglas, luego memoria, y el LLM como última opción. Siempre puedes forzar un modelo.
- **Modular y frugal.** Biblioteca estándar primero, extras opcionales, proveedores, almacenes y frontends reemplazables.
- **Terminal primero.** Validar el núcleo antes de añadir cualquier GUI.
- **Adaptable.** Corre en un portátil modesto con proveedores en la nube, o en una estación con modelos locales.

## Plan de versiones

| Versión | Enfoque | Contenido |
|---|---|---|
| **0.1.0** | Base segura | Núcleo modular, proveedores unificados (compatible con OpenAI, Google), vault de keyring con niveles, redacción y deny globs, permisos por riesgo, **sandbox** (bubblewrap), herramientas (archivos, shell en sandbox, git), REPL en terminal, LSP para Kate (hover y completado), `kellu doctor` |
| **0.2.0** | Agente | Loop de agente con herramientas y confirmaciones, etapa de reglas de la cascada, inyección de contexto |
| **0.3.0** | Interoperabilidad | Gateway local compatible con OpenAI, modo agente ACP |
| **0.4.0** | Memoria | Almacén de soluciones aceptadas (SQLite por defecto, PostgreSQL + pgvector opcional), etapa de memoria |
| **0.5.0** | Adaptación | Detección de runtimes locales (Ollama, LM Studio, llama.cpp), perfiles de hardware, servidor y cliente MCP |
| **Después** | | GUI mediante plugins existentes de Kate; backends de sandbox Landlock, macOS y Windows |

0.1.0 **no incluye intencionalmente un loop de agente autónomo**: las herramientas, los permisos y el sandbox se validan por separado antes de que un modelo pueda dispararlos.

## Arquitectura

```
 Terminal   Kate (LSP)   Gateway*   ACP*   MCP*          <- frontends
      \          |          |        |      /
 ========= Núcleo de Kellu (confiable: claves, red) =========
  scope -> reglas* -> memoria* -> sanitizador -> router -> proveedor
                                            \-> herramientas (niveles de riesgo, permisos)
 =============================================================
         |                                   |
  sandbox (bubblewrap):               lado host, con confirmación:
  shell, tests, formateadores         escrituras de git y gh

  * posterior a 0.1.0
```

El núcleo no tiene interfaz ni efectos secundarios al importarse. Frontends, proveedores y almacenes son adaptadores reemplazables. Dirección de dependencias: `frontends → core → tools → sandbox, security → common`, con `providers` y `projects` también por debajo de `core` (ver [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)). Un test lo hace cumplir.

## Seguridad de un vistazo

- **Sandbox.** Las herramientas que lanzan procesos corren en bubblewrap: sin red por defecto, sistema en solo lectura, el proyecto como único directorio escribible, entorno depurado y sin acceso a `~/.ssh`, `~/.gnupg` ni al bus del keyring.
- **Niveles de claves.** Las claves `provider` están disponibles en el flujo normal. Los secretos `infra` y `db` (nube, DNS, bases de datos) no son accesibles para el asistente salvo habilitación explícita con confirmación.
- **Niveles de riesgo.** Cada herramienta es `read`, `write`, `destructive` o `network`. Las acciones destructivas requieren confirmación escrita. Las escrituras de git y GitHub corren en el host, nunca dentro del sandbox.
- **Salida de datos.** Los deny globs (`.env`, claves, certificados) y la redacción de secretos se aplican antes de enviar algo a un proveedor. Se planea un modo solo local por proyecto.
- **Un entorno virtual de Python aísla dependencias, no la seguridad.** Lo que restringe las herramientas es el sandbox.

Detalles, modelo de amenazas y límites: [SECURITY.md](SECURITY.md).

## Cascada de decisión (diseño, 0.2.0 a 0.4.0)

1. **Reglas.** Manejo determinista que no necesita modelo (detección de stack, estado de git, comandos conocidos).
2. **Memoria.** Busca soluciones aceptadas previamente. Se reutiliza solo si la aceptaste o pasaron los tests, supera un umbral de similitud y no ha caducado. Las respuestas reutilizadas se etiquetan "desde memoria".
3. **LLM.** Enruta por tipo de tarea al mejor proveedor, con respaldo. Fuerza un modelo con `--model` o una opción equivalente.

## Proyectos y scopes

Los proyectos se descubren bajo una raíz configurable y se agrupan en macro-proyecto, repositorio y subproyecto. Cada scope puede definir reglas, proveedores preferidos, límites de recursos y un modo de privacidad (por ejemplo, solo local).

## Agentes y extensiones

Kellu hablará los protocolos abiertos para agentes y herramientas (ACP y MCP, entre 0.3.0 y 0.5.0). Los agentes y servidores MCP, propios o de terceros, se tratan como **extensiones no confiables**: corren como procesos en sandbox con entorno depurado, se habilitan solo mediante una lista de permitidos explícita por servidor, y cada herramienta que exponen necesita un nivel de riesgo declarado. No hay código por agente dentro del núcleo; un único cliente genérico los atiende a todos.

Ideas en evaluación, como repositorios separados y fuera de 0.1.0:

- **kellu-crawler**: crawler web enfocado a proyectos e investigación
- **kellu-ocr**: extracción de texto (OCR) para PDF escaneados antiguos y documentos similares

Los resultados se entregarían a la terminal o como archivos; la entrega dentro de Kate depende de lo que su cliente LSP pueda mostrar.

## Estructura del proyecto

```
kellu/
├── LICENSE  README.md  README.es.md  SECURITY.md  CHANGELOG.md  pyproject.toml
├── src/kellu/
│   ├── __app__.py          # identidad: nombre, versión, esquema del keyring (fuente única)
│   ├── cli.py  doctor.py   # puntos de entrada: kellu, doctor, keys, lsp...
│   ├── common/             # errores, rutas XDG (solo biblioteca estándar)
│   ├── security/           # vault, redacción, deny globs, permisos y política
│   ├── sandbox/            # interfaz, backend bubblewrap, perfiles
│   ├── providers/          # compatible OpenAI, Google (runtimes locales después)
│   ├── projects/           # descubrimiento, detección de stack, scopes
│   ├── tools/              # registro con nivel de riesgo obligatorio; archivos, shell, git
│   ├── core/               # config, router, contexto, sesiones (sin efectos al importar)
│   └── frontends/          # REPL, LSP (después: gateway, ACP, servidor MCP)
├── tests/                  # unit/ y sandbox/
├── docs/                   # ARCHITECTURE.md y notas de diseño
└── packaging/              # entrada de escritorio
```

Los paquetes se añaden **cuando empieza su hito**, no antes: `memory/` (0.4.0, SQLite y pgvector opcional), la cascada de decisión como `core/cascade.py` (0.2.0 a 0.4.0) y un cliente MCP dentro de `tools/` (0.5.0). Un test exige que cada directorio sea una capa declarada y que las capas solo importen hacia abajo.

## Inicio rápido

> Los comandos describen la **interfaz objetivo** y pueden cambiar antes de 0.1.0.

Requisitos (Linux primero): Python 3.11+, `libsecret-tools` (`secret-tool`) y `bubblewrap`. En Debian o Ubuntu:

```
sudo apt install libsecret-tools bubblewrap
```

Algunas distribuciones restringen los user namespaces sin privilegios (por ejemplo Ubuntu 24.04); `kellu doctor` te dice si el sandbox puede ejecutarse.

```
# Una vez publicado
pipx install kellu

# Desarrollo
git clone https://github.com/<tu-usuario>/kellu.git
cd kellu
python -m venv .venv && source .venv/bin/activate
pip install -e ".[dev]"      # extras opcionales: [tui], [local], [dev]

kellu doctor                 # revisa keyring, bubblewrap y user namespaces
kellu keys set openrouter    # se guarda en tu keyring, sin mostrarse en pantalla
kellu                        # asistente en terminal
```

Si tienes secretos en un esquema anterior del keyring, `kellu keys migrate --dry-run` lista nombres (nunca valores) y la copia jamás borra los originales.

### Úsalo dentro de Kate (LSP)

En Kate: *Preferencias → Configurar Kate → Cliente LSP → Configuración de servidores de usuario*. Ejemplo:

```json
{
  "servers": {
    "python": {
      "command": ["kellu", "lsp"],
      "highlightingModeRegex": "^Python$"
    }
  }
}
```

El servidor LSP de Kellu ofrece solo **hover** y **completado**. Revisa la documentación del cliente LSP de Kate para el formato exacto en tu versión.

## Configuración

| Ruta | Propósito |
|---|---|
| `~/.config/kellu/` | Proveedores, enrutamiento, scopes, política de sandbox |
| `~/.local/state/kellu/` | Registros de sesión, historial (redactados, modo 0700) |
| `~/.cache/kellu/` | Cachés |

La configuración es un único archivo TOML, `~/.config/kellu/config.toml`, combinado sobre los defaults empaquetados. Nunca contiene secretos (usa el keyring), los endpoints deben usar HTTPS (HTTP solo para localhost), las claves desconocidas son errores, y el archivo debe pertenecerte y no ser escribible por grupo ni otros. Revísala con `kellu config path` y `kellu config check`.

```toml
[routing.code]
primary = "deepseek"
backup = ["openrouter"]

[routing.general]
primary = "google"
backup = ["deepseek", "openrouter"]
```

## Perfiles de hardware (objetivo de diseño)

| Perfil | Objetivo | Qué corre |
|---|---|---|
| `minimal` | cualquier equipo | núcleo + terminal, proveedores en la nube, SQLite |
| `standard` | portátil habitual | además LSP y MCP, modelo local pequeño para completado |
| `full` | RAM o GPU holgadas | además integración con plugins de Kate, modelos locales mayores, PostgreSQL + pgvector |

## Hitos hacia 0.1.0

- [x] **M0** Repositorio, layout `src/`, `pyproject.toml`, rutas XDG, CI, tests, licencia, `SECURITY.md`
- [x] **M1** Configuración con defaults empaquetados, errores, logging, sesiones; sin efectos al importar
- [ ] **M2** Proveedores: cliente compatible con OpenAI (streaming, herramientas), Google, router con timeouts y respaldo
- [ ] **M3** `kellu keys` con niveles y migración, entorno depurado, deny globs, redacción v1, permisos por riesgo
- [ ] **M4** Sandbox: interfaz, backend bubblewrap, políticas, `kellu doctor`, tests de escape
- [ ] **M5** Herramientas con esquemas generados y niveles de riesgo: archivos, shell en sandbox, git de solo lectura; escrituras de git y gh en el host con confirmación
- [ ] **M6** REPL en terminal sobre el núcleo; LSP para Kate (hover, completado)
- [ ] **M7** Documentación y etiqueta 0.1.0

## Proyectos relacionados

Kellu es independiente, pero está pensado para interoperar con otros esfuerzos de IA para Kate en lugar de reemplazarlos:

- [KateAI](https://github.com/ObiWindu/KateAI): plugin de Kate con una experiencia de agente completa en GUI.
- [kate-agents](https://github.com/Mte90/kate-agents): plugin de Kate que lleva agentes al estilo Zed al editor.

La interoperabilidad es una meta, no una integración existente. No se implica respaldo de sus autores.

## Sobre el nombre

*Kellu* significa "ayudante" en mapudungun, la lengua del pueblo mapuche. El nombre refleja el propósito del proyecto: un asistente que ayuda, en silencio, dentro de las herramientas que ya usas.

## Contribuciones

El proyecto es joven y la arquitectura aún se mueve, así que abre un issue antes de enviar cambios grandes. Se agradecen especialmente: revisión de seguridad, sanitización, abstracciones de proveedores, integración LSP y esquemas de datos para memoria orientada a desarrollo. **Reporta las vulnerabilidades en privado**, como se describe en [SECURITY.md](SECURITY.md).

## Licencia

MIT. Ver [LICENSE](LICENSE).

## Aviso legal

Kate es un editor de texto desarrollado por la comunidad KDE. Kellu es un proyecto independiente y no está afiliado, patrocinado ni respaldado por KDE. La salida generada por IA puede ser incorrecta: revisa las sugerencias antes de aplicarlas.
