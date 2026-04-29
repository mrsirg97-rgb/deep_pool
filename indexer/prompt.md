# deep_pool indexer

we are going to use a little trick to make this lightning fast. the schema in ../db/01-schema.sql will be our starting point. a challenge for this, no in db joins or views. the indexer itself will handle the joins in memory using a service based architecture that forms a dag.

indexer/
-- domain/ ~ this is where our db transactions will live
-- services/ ~ this is where our service layer lives that will utilize the domain.

transaction should be created at the route/api layer, which we can define after. but the services will essentially define the relationships. for instance:

```
pools domain methods:
    set(pool_entries: pool entries) -> pool_entries, this allows us to broadcast as well
    get(pool_id: int) -> pool entry
    list(condition?: Option<string>) -> pool entries
    del(pool_ids: int[]) -> bool

pools service will internally call these entries with something like
    get_token_mints_pools(mints: string[]) -> internally builds the condition, then calls domain list
    get_creators_pools(creator: string[])
    

similar for other services, but we will use composition as well

so lets say i want to get swaps through the swap service:
    1.) swap service injects pool service
    2.) call swap get price data looking up mint
    3.) delegate to pool service, return back pool entries, map to pool ids, pass as where condition to swap domain list

for complex joins, we can cache intermediate results per request. we will also have the transaction be open for the life of the request, so it will handle the query chain as a single tx.
```

use ~/Projects/metadao-challenge/indexer for laserstream integration. we will be indexing the events emitted from the new deep_pool events.