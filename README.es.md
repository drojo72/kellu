<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Kellu 🤖

**Kellu** es un asistente de IA modular, con enfoque *local-first* y CLI con soporte para Language Server Protocol (LSP), diseñado para desarrolladores que exigen privacidad, velocidad y una integración nativa con el llavero del sistema operativo.

Evolucionado desde un prototipo inicial en Python hacia una arquitectura de alto rendimiento basada en **Ruby Core / Extensiones en Rust**, Kellu orquesta interacciones con múltiples proveedores de IA manteniendo las credenciales estrictamente aseguradas dentro del llavero del sistema Linux.

---

## 📸 Características Principales

- **Seguridad Local-First**: Integración nativa con el servicio de secretos de Linux (`secret-tool`) bajo el esquema `org.freedesktop.Secret.Generic`. No se almacenan claves en texto plano.
- **CLI Robusta**: Interfaz modular por línea de comandos impulsada por Thor con aislamiento de subcomandos.
- **Agregación de Proveedores**: Interfaz unificada de enrutamiento para consultar múltiples backends de IA (DeepSeek, OpenRouter, Google AI, Cloudflare R2, etc.).
- **Entorno Aislado con Bundler**: Gemas empaquetadas localmente bajo Ruby 3.2+ sin contaminar el entorno global.
- **Rendimiento Nativo**: Núcleo ágil con extensiones planificadas en Rust (`ext/`) para procesamiento intensivo de texto.

---

## 🏗 Arquitectura del Sistema y Árbol de Directorios

```text
kellu/
├── assets/
│   └── branding/              # Recurso de marca en SVG y PNG
├── bin/
│   └── kellu                   # Ejecutable principal de la CLI (Punto de entrada Thor)
├── docs/
│   ├── ARCHITECTURE.md         # Definición de capas y reglas de dependencias
│   ├── bitacora/               # Registro de sesiones de desarrollo e índice
│   ├── branding/               # Guías de marca, tipografía y mockups
│   └── design/                 # Diseños de sandbox y capacidades LSP
├── ext/                        # Extensiones C/Rust para rendimiento nativo
├── lib/
│   ├── kellu/
│   │   ├── core/               # Motor de configuración y enrutador de proveedores
│   │   ├── database/           # Almacenamiento local de estado
│   │   ├── providers/          # Adaptadores para proveedores de IA
│   │   ├── security/           # Gestor del llavero del sistema (secret-tool)
│   │   ├── cli.rb              # Definición de subcomandos CLI con Thor
│   │   ├── version.rb          # Versión de la gema Kellu (v0.1.0)
│   │   └── error.rb            # Jerarquía de errores personalizados
│   ├── tools/                  # Herramientas de ejecución aisladas
│   └── kellu.rb                # Punto de entrada del módulo raíz
├── test/                       # Suite de pruebas
├── vendor/
│   └── bundle/                 # Gemas locales empaquetadas
├── Gemfile                     # Definición de dependencias del proyecto
└── README.md                   # Documentación del proyecto
```

---

## 🚀 Inicio Rápido

### Requisitos Previos

- **Ruby**: 3.2.3 o superior
- **Herramientas del Sistema**: `build-essential`, `ruby-dev`, `libsecret-1-0`, `secret-tool`

### Instalación y Configuración

1. **Clonar el repositorio**:
   ```bash
   git clone https://github.com/tu-usuario/kellu.git
   cd kellu
   ```

2. **Instalar dependencias localmente**:
   ```bash
   bundle config set --local path 'vendor/bundle'
   bundle install
   ```

3. **Verificar el ejecutable**:
   ```bash
   bundle exec bin/kellu version
   ```

---

## 🔑 Gestión de Credenciales

Kellu almacena las credenciales de forma segura en el llavero del sistema bajo los atributos `kai` / `kellu`.

```bash
# Listar subcomandos disponibles
bundle exec bin/kellu secret help

# Guardar una clave de forma segura
bundle exec bin/kellu secret set ai_gemma_direct_api_key "tu_clave_api_secreta"

# Leer una credencial almacenada
bundle exec bin/kellu secret get ai_gemma_direct_api_key

# Eliminar una credencial del llavero
bundle exec bin/kellu secret delete ai_gemma_direct_api_key
```

---

## 🗺 Hoja de Ruta del Proyecto

- [x] **Fase 1: Arquitectura y Base de Seguridad**
  - [x] Migración desde el prototipo heredado en Python hacia la arquitectura Ruby/Rust.
  - [x] Estructura CLI con Thor y manejo de excepciones limpio.
  - [x] Gestor nativo `KeyringManager` mediante `Open3` y `secret-tool`.
  - [x] Auditoría y migración de credenciales del sistema.

- [/] **Fase 2: Configuración y Enrutador de Proveedores (En Progreso)**
  - [ ] Gestión de configuración TOML en `lib/kellu/core/config.rb`.
  - [ ] Enrutador de API multiproveedor en `lib/kellu/core/router.rb`.
  - [ ] Cargador de prompts de sistema y plantillas.

- [ ] **Fase 3: Extensiones Nativas en Rust y Servidor LSP**
  - [ ] Integración de extensiones en Rust para procesamiento pesado de texto.
  - [ ] Implementación del protocolo LSP para integración con editores (Kate / Neovim / VS Code).

---

## 📄 Licencia

Licencia MIT. Consulta `LICENSE` para más detalles.
