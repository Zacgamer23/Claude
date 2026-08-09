# Generador de perfiles personales

Una página web de un solo archivo que redacta **usernames, nombres visibles y bios**
para varias cuentas de Instagram que pertenecen a **la misma persona real**, cada una
con su propio tema.

Abre `index.html` en cualquier navegador. No necesita instalación, servidor ni conexión.

## Qué hace

Introduces tu nombre base y unos cuantos ajustes, y genera para cada cuenta:

- **Username** (`@…`), entre 4 y 20 caracteres, solo `a-z 0-9 . _`, sin repetirse
- **Nombre visible**, una variación natural de tu nombre
- **Bio** en primera persona, con el límite real de Instagram de 150 caracteres
- **Enfoque y personalidad**, dos o tres frases sobre el ángulo de esa cuenta
- **Entre 5 y 8 cuentas públicas reales** que encajan con ese tema

Debajo aparecen una tabla resumen y unas notas sobre cómo llevar varios perfiles a mano.
Todo se puede copiar como Markdown o JSON, o descargar como `.md`.

## Ajustes

| Ajuste | Qué cambia |
|---|---|
| Nombre base | La raíz de todos los usernames y nombres visibles |
| Nº de cuentas | De 1 a 12 |
| Idioma de bios | Español o inglés |
| Ciudad | Si la pones, aparece en las bios |
| Tono | Mínimo, cálido, seco o juguetón — cambia cómo se redacta la bio |
| Temas | Elige tú los temas o deja que los reparta |
| Emojis | Añade un emoji al principio de cada bio |
| Cuentas enlazadas | Añade «Principal: @…» a las bios secundarias |

Los resultados son deterministas: con los mismos ajustes salen los mismos perfiles.
**Otra variante** genera una alternativa distinta con los mismos ajustes.

## Alcance

La página genera **solo texto**. No se conecta con Instagram, no crea cuentas, no
inicia sesión, no sigue a nadie y no automatiza nada. Lo que hagas con el texto lo
haces tú, a mano.

Las cuentas sugeridas para seguir son perfiles públicos y conocidos (medios, marcas,
museos, revistas). Los usernames cambian de dueño con el tiempo, así que conviene
buscarlos dentro de la app y confirmarlos antes de seguirlos.

Está pensada para el caso honesto: una persona con varias cuentas propias, cada una
con un tema distinto. No reparte una misma cuenta objetivo entre todos los perfiles
ni intenta que parezcan personas diferentes.

## Estructura

Un único archivo, `index.html`, sin dependencias externas: HTML, CSS y JavaScript
en vanilla. Los temas y las cuentas sugeridas están en el objeto `NICHES`, al
principio del `<script>` — se pueden añadir o editar ahí.
