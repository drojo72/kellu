<div align="center">
  <img src="/assets/branding/svg/kellu-logo-color-dark.svg" alt="Kellu Logo" width="350" />
  <p><em>Asistente de IA modular, local-first y CLI LSP construido con Ruby y Rust</em></p>
</div>

---

# Registro de Cambios (Changelog)

Todos los cambios notables en el proyecto **Kellu** serán documentados en este archivo.

El formato está basado en [Keep a Changelog](https://keepachangelog.com/es-ES/1.0.0/) y este proyecto se adhiere a [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [0.1.0] - 20-10-2026

### Añadido
- Migración completa de arquitectura desde el prototipo heredado en Python hacia el núcleo en **Ruby 3.2+ y extensiones nativas en Rust**.
- Estructura CLI modular impulsada por `Thor` con subcomandos aislados (`bin/kellu`).
- Envoltorio nativo del servicio de secretos de Linux en `lib/kellu/security/keyring.rb` utilizando `Open3` y `secret-tool`.
- Parser robusto por bloques (`[/N]`) para gestionar estructuras e índices del llavero sin fallos al recortar cadenas.
- Suite completa de identidad visual: logotipos vectoriales, guía tipográfica y mockups de terminal en `/assets` y `/docs/branding`.
- Estructura de documentación del proyecto: `ARCHITECTURE.md`, `SECURITY.md` y bitácora de sesiones en `/docs/bitacora/`.

### Corregido
- Resolución de advertencias de sintaxis en Thor mediante configuraciones explícitas de descripción y `self.exit_on_failure? = true`.
- Corrección de errores de carga automática (*autoload*) en plantillas de `lib/kellu/core/config.rb` y `lib/kellu/core/router.rb`.
- Depuración y corrección de la clave corrupta `ai_gemma_direct_api_key` originada por desajustes de atributos en el sistema heredado.

### Cambiado
- Estandarización simétrica para la eliminación y lectura de secretos utilizando etiquetas y atributos correspondientes al esquema `kai` / `kellu`.