// La captura usa V4L2 directamente (crate `v4l`), no el SDK propietario
// `lib3DCamera.so` (que exige licencia por número de serie y bloqueaba la
// conexión). Por eso ya no enlazamos ninguna librería nativa: este build script
// es un no-op.
//
// El código del SDK por FFI se conserva en `src/sdk/` (fuera del árbol de
// módulos en `main.rs`) como referencia, por si en el futuro se opta por la vía
// con licencia.
fn main() {}
