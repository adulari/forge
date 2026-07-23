# Reservations reference

The fixture begins with ten failures involving async overselling, invalid quantity handling,
conflicting request IDs, rollback, cancellation idempotency, and stable ordering. The saved reference
passes all eight test methods:

```bash
cd reference
python3 -m unittest discover -v
```
