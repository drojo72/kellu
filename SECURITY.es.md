<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcet="/assets/branding/svg/kellu-logo-color-dark.svg">
    <img alt="Kellu" src="/assets/branding/svg/kellu-logo-color-light.svg" width="350">
  </picture>
    <em>Local-first, modular AI assistant & LSP CLI built with Ruby & Rust</em>
</p>

---

# Política y Arquitectura de Seguridad

## Principios de Seguridad

Kellu está diseñado bajo un modelo de cero confianza para entradas no seguras, respuestas del modelo y extensiones de terceros. Los principios fundamentales son:

1. **Cero Credenciales en Texto Plano**: Ninguna clave API, contraseña o token se almacena en disco o archivos de configuración sin cifrar.
2. **Aislamiento en Llavero (Keyring)**: Las credenciales residen exclusivamente en el servicio de secretos de Linux, accesibles mediante `secret-tool` bajo el esquema `org.freedesktop.Secret.Generic`.
3. **Ejecución Segura con Fallo Cerrado (Fail-Closed)**: Cualquier fallo de permisos, intento fuera del alcance (*scope*) o inconsistencia de hash detiene la operación inmediatamente.
4. **Operaciones en Aislamiento (Sandbox)**: La ejecución de procesos y modificaciones de archivos activadas por herramientas de IA corren en entornos restringidos.

---

## Almacenamiento de Credenciales (`KeyringManager`)

- **Backend**: Interfaz DBus de XDG Secret Service en Linux ejecutada vía `Open3` de Ruby.
- **Atributos**: El esquema coincide con `org.freedesktop.Secret.Generic` con identificadores de servicio `kai` / `kellu`.
- **Parsing**: El análisis por bloques línea por línea (`[/N]`) garantiza una extracción precisa mediante etiquetas (*labels*) sin alterar nombres de servicios compuestos.

---

## Restricciones de Ámbito y Rutas (*Scope*)

- **Contención en la Raíz**: Todas las operaciones sobre archivos están limitadas a la raíz del proyecto. Se rechaza el acceso a `/`, `$HOME` o directorios superiores del usuario.
- **Lista de Denegación Automática**: Queda estrictamente prohibido el acceso a archivos sensibles:
  - Credenciales y archivos de entorno: `.env*`, `*.pem`, `*.key`, `id_*`, `credentials*`, `*.token`.
  - Internos del control de versiones: `.git/`.
- **Resolución**: La resolución de rutas utiliza descriptores de directorio (`O_NOFOLLOW`) para prevenir ataques de salto de enlaces simbólicos (ej. CVE-2026-87766).

---

## Entorno Aislado y Seguridad de Ejecución

- **Aislamiento con Bubblewrap**: Las herramientas que ejecutan procesos se aíslan mediante `bubblewrap` usando `--unshare-all`, limpiezas de entorno, montajes de sistema de solo lectura y sistemas de archivos temporales para `/tmp` y `$HOME`.
- **Copias de Trabajo Temporales (Staging)**: Las herramientas que solicitan modificaciones trabajan sobre copias temporales. Los cambios se validan mediante diffs de propuestas antes de ser aplicados al repositorio tras aprobación explícita.

---

## Reporte de Vulnerabilidades

Si descubres una vulnerabilidad de seguridad en Kellu, por favor repórtala de forma privada por correo electrónico o mediante una notificación de seguridad privada en el repositorio. No abras reportes públicos para problemas de seguridad sensibles.
