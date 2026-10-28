<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Arquitectura y Diseño del Sistema

Este documento detalla la arquitectura de software, los principios de diseño, la jerarquía de capas y las fronteras de seguridad para **Kellu**.

## 1. Visión General de la Arquitectura y Principios de Diseño

Kellu está diseñado como un **asistente de IA modular local-first y una herramienta CLI basada en el Protocolo de Servidor de Lenguaje (LSP)**. La arquitectura prioriza la seguridad del sistema, la ejecución local, las dependencias explícitas y la extensibilidad modular.

### Principios Fundamentales

* **Seguridad Local-First**: Las credenciales y los estados residen exclusivamente dentro de los esquemas del sistema operativo local (p. ej., el Servicio de Secretos de Linux mediante `secret-tool`). No se registran ni se escriben credenciales en archivos de texto plano.

* **Aislamiento Estricto por Capas**: Las dependencias fluyen estrictamente hacia abajo en un Grafo Acíclico Dirigido (DAG). Los orquestadores de alto nivel pueden depender de utilidades de menor nivel, pero las capas inferiores nunca hacen referencia a las capas superiores.

* **Modelo Híbrido Políglota**: La orquestación de alto nivel, el parseo de la CLI y la lógica de flujos de trabajo se gestionan en **Ruby** por su expresividad y rapidez de iteración. Las operaciones críticas para el rendimiento, de baja latencia o de procesamiento intensivo de texto se delegan a **extensiones nativas compiladas en Rust** (`ext/`).

* **Ejecución en Entorno Aislado (Sandbox)**: Las herramientas externas y los comandos agénticos se ejecutan bajo políticas estrictas de sandbox para evitar efectos secundarios no autorizados en el sistema anfitrión.

---

## 2. Arquitectura en Capas (Núcleo Ruby)

El código fuente en Ruby bajo `lib/kellu/` está estructurado en capas funcionales bien definidas. Las capas superiores pueden invocar componentes de capas equivalentes o inferiores, pero las capas inferiores deben mantenerse completamente desacopladas e ignorantes de las capas superiores.

```text
+---------------------------------------------------------------+
|                      Capa Frontends                           |
|         (bin/kellu, CLI / Thor, Servidor LSP, REPL)           |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                        Capa Core                              |
|        (Motor Config, Prompts Sistema, Enrutador, Estado)    |
+---------------------------------------------------------------+
               |                 |                 |
               v                 v                 v
+-----------------------+ +-------------+ +---------------------+
|     Capa Projects     | | Capa Tools  | |   Capa Providers    |
| (Workspace, Indexado) | | (Ejecutores)| | (DeepSeek, Open3)   |
+-----------------------+ +-------------+ +---------------------+
               |                 |                 |
               +-----------------+-----------------+
                               |
                               v
+---------------------------------------------------------------+
|                       Capa Sandbox                            |
|             (Aislamiento Procesos, Políticas)                 |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                      Capa Security                            |
|        (Gestor de Llavero, Wrapper secret-tool / Open3)       |
+---------------------------------------------------------------+
                               |
                               v
+---------------------------------------------------------------+
|                       Capa Common                             |
|             (Errores Base, Utilidades, Logger)                |
+---------------------------------------------------------------+
```

### Desglose de Capas

1. **`Frontends` (`lib/kellu/cli.rb`, futuro `lib/kellu/lsp/`)**
   * **Rol**: Puntos de entrada para usuarios humanos o interfaces de editores (VS Code, Neovim, Kate).
   * **Responsabilidades**: Parseo de comandos (Thor), formateo de salida, manejo de señales y traducción entre las peticiones externas del usuario y las operaciones de `Core`.

2. **`Core` (`lib/kellu/core/`)**
   * **Rol**: Cerebro y orquestador del sistema.
   * **Responsabilidades**: Ciclo de vida de la aplicación, lectura de configuración TOML (`config.rb`), enrutamiento de peticiones (`router.rb`), carga de plantillas de prompts y gestión del estado.

3. **`Projects` (`lib/kellu/projects/`)**
   * **Rol**: Gestor de contexto y espacio de trabajo.
   * **Responsabilidades**: Análisis estructural del código base, indexación local de archivos, recuperación de contexto y contexto a nivel de repositorio.

4. **`Tools` (`lib/kellu/tools/`)**
   * **Rol**: Definiciones de herramientas agénticas e implementación de capacidades.
   * **Responsabilidades**: Rutinas de ejecución para código, modificación de archivos, comandos de terminal e inspección de APIs.

5. **`Providers` (`lib/kellu/providers/`)**
   * **Rol**: Adaptadores para proveedores de IA.
   * **Responsabilidades**: Estandarización de APIs externas (DeepSeek, OpenRouter, Google AI, LLMs locales) a un formato interno unificado de mensajes.

6. **`Sandbox` (`lib/kellu/sandbox/`)**
   * **Rol**: Contexto de ejecución segura.
   * **Responsabilidades**: Aplicar límites de ejecución, restricciones de entorno y verificaciones de políticas antes de que `Tools` o comandos ejecuten procesos del sistema.

7. **`Security` (`lib/kellu/security/`)**
   * **Rol**: Gestor de credenciales y secretos (`keyring_manager.rb`).
   * **Responsabilidades**: Comunicación segura con `secret-tool` a través de `Open3`, aislamiento de tokens en memoria y protección contra fugas de claves en registros o trazas de error.

8. **`Common` (`lib/kellu/error.rb`, `lib/kellu/version.rb`)**
   * **Rol**: Primitivas fundamentales compartidas.
   * **Responsabilidades**: Jerarquía de excepciones personalizadas (`Kellu::Error`), abstracciones de logs y constantes globales.

---

## 3. Reglas de Dependencias e Importación

Para prevenir el acoplamiento y las dependencias cíclicas, Kellu impone estrictamente las siguientes reglas:

1. **Flujo Exclusivamente Descendente**: Las importaciones de módulos (`require_relative`) solo pueden apuntar a componentes de capa equivalente o inferior.
2. **Sin Referencias Ascendentes**: Las capas inferiores (p. ej., `Security`, `Common`) **nunca** deben hacer referencia a `Core`, `CLI` o `Providers`.
3. **Aislamiento de Proveedores**: Los adaptadores dentro de `lib/kellu/providers/` deben estar desacoplados entre sí. Se comunican únicamente mediante interfaces estandarizadas provistas por `Core` o `Common`.
4. **Propagación Limpia de Errores**: Las excepciones generadas en capas inferiores deben heredar de `Kellu::Error` (definido en `Common`) y propagarse hacia arriba para ser capturadas en `Frontends` / `CLI`.

---

## 4. Extensiones Nativas en Rust (`ext/`)

Para aquellas operaciones donde la velocidad de ejecución de Ruby o las limitaciones del GIL afecten el rendimiento, se integran extensiones nativas en Rust en el directorio `ext/kellu_native/`.

### Casos de Uso para Extensiones en Rust

* **Tokenización y Parseo de Alta Velocidad**: Procesamiento de grandes ventanas de contexto dinámico y código fuente para el servidor LSP.
* **Cálculo de Diff y Vectores**: Búsqueda local por similitud vectorial y cálculo estructural de diferencias.
* **Serialización de Protocolo**: Parseo ultra rápido de JSON-RPC para la comunicación de alto rendimiento del servidor LSP.

### Frontera FFI / Extensión C

* **Magnus / rb-sys**: Se utilizan como puente entre la API en C de Ruby y Rust.
* **Garantía de Seguridad**: La memoria asignada dentro de las extensiones en Rust permanece aislada; las operaciones de memoria inválidas en Rust generan un *panic* controlado en la frontera de la extensión en lugar de un *segmentation fault* no capturado en el proceso de Ruby.

---

## 5. Fronteras de Confianza y Modelo de Seguridad

```text
[ ZONA NO CONFIABLE ] ────────> [ ZONA CONTROLADA ] ────────> [ ZONA LOCAL CONFIABLE ]
   API de IA Externa            Adaptadores Kellu              Servicio de Secretos
 (DeepSeek, OpenRouter)       y Sanitizadores Entrada         (secret-tool / Llavero)
         │                               │                               │
         └───────── Cifrado ─────────────┴────────── Flujo ──────────────┘
                    TLS Estricto                  Memoria Open3
```

1. **Frontera de Credenciales del Sistema**:
   * Los secretos se obtienen bajo demanda desde `secret-tool` directamente hacia la memoria de Ruby.
   * Queda estrictamente prohibido persistir claves en disco (`.env`, archivos TOML de configuración o archivos temporales).

2. **Frontera de Datos Externos**:
   * Las respuestas recibidas de los LLMs externos se tratan como **datos no confiables**.
   * Los comandos generados por los modelos deben pasar por el motor de políticas de `Sandbox` antes de ser evaluados para su ejecución.

3. **Frontera de Ejecución**:
   * La ejecución de herramientas es explícita. Los comandos destructivos requieren autorización previa del usuario o reglas estrictas dentro del sandbox.

---

## 6. Hoja de Ruta de Integración de Componentes Futuros

* **Componente Servidor LSP (`lib/kellu/lsp/`)**: Se acoplará a la capa `Frontends`, exponiendo las capacidades de Kellu a editores mediante stdio/sockets utilizando JSON-RPC.
* **Base de Datos Local SQLite / Vectorial (`lib/kellu/database/`)**: Se integrará entre `Projects` y `Core` para almacenar en caché los embeddings del código base local y el historial de conversaciones de forma segura.
* **Motor de Texto en Rust**: Sustitución gradual de los analizadores de Ruby que presenten cuellos de botella por métodos en `ext/kellu_native` sin alterar los contratos públicos de Ruby.
