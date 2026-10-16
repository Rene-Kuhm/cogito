# Limitaciones conocidas

Estado de lo medido. Cada número tiene su informe en `odd/tasks/`.

## Consultas en español sobre código en inglés

Es la limitación más grande que conocemos. En el holdout privado de un proyecto real (24 tareas), el recall es de 29,2 % en general, 66,7 % en inglés y **16,7 % en español**.

Probamos cuatro cosas, y ninguna lo mejoró en el holdout de español sin empeorar otro subconjunto:

- Glosario de términos español → inglés: sin efecto en el set público, −13,4 pts en tareas nuevas. Revertido.
- Embeddings con un modelo en inglés (bge-small) y con uno de código cuantizado: español igual (50 % → 50 %). Ver `odd/tasks/embeddings-b2-v1.md`.
- Modelo multilingüe pequeño (E5-small, 118 MB): español igual en holdout y tuning; inglés empeora en tuning. Ver `odd/tasks/es-multilingual-v1.md`.
- Stopwords y stemming en español: español del holdout baja de 50 % a 25 %. Ver `odd/tasks/es-lexical-v1.md`.

**Recomendación para quien use cogito:** consultar en inglés cuando sea posible. La tarjeta del proyecto y los nombres de símbolos están en inglés.

## Impacto dentro de `context_pack` (C3)

Implementado detrás de un flag apagado, y no incluido en `main`. En los sets medidos no empeora ni mejora: el flag no cambia los resultados. No hay evidencia de beneficio. Ver `odd/tasks/impact-pack-v1.md` en `feat/impact-in-pack`.

## Diff al releer un archivo (D5)

Verificado y funciona con el binario, pero no ahorra nada en las sesiones medidas: no hubo relecturas. No incluido en `main`. Ver `odd/tasks/d3-replay-v1.md` en `feat/d3-replay`.

## Dónde van los tokens

Medido en 128 sesiones de cuatro agentes (Claude Code, Gentle Shell, Codex y opencode). Ver `odd/tasks/token-sources-v1.md`.

- Las relecturas de archivos completos son pocas en el uso real. La mayor parte del costo es la longitud de la sesión: cada turno vuelve a leer todo el contexto.
- Un modelo concentró el costo en Gentle Shell: `MiniMax-M2.7` pagaba la entrada completa en cada turno. El modelo por defecto pasó a `MiniMax-M3.1-Flash-Preview`.
- El recorte de salidas grandes de Bash existe pero está apagado por defecto. Solo cubre Claude Code, y en las sesiones medidas afecta al 1,9 % de las llamadas.

## Lo que cogito no cubre

- Agentes sin sistema de hooks (por ejemplo Codex, según lo que medimos): no hay forma de bloquear sus lecturas desde afuera. Solo puede ayudar con una herramienta MCP y con instrucciones.
- Gentle Shell y opencode aplican el guardrail de lectura a través de `cogito guard`, pero no todos los eventos de esos agentes tienen su equivalente al recorte de salidas.
