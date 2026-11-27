# Bluefin library search ordering

Bluefin uses the existing user-scoped Items request, including its visibility, parental, tag and collection filters.

A request with SearchTerm and no SearchOrder retains Jellyfin's relevance-first order. SortBy and SortOrder keep their existing Jellyfin meaning unless the request opts into the extension.

For an explicit user-selected library sort, send:

    SearchTerm=blade&SearchOrder=SortBy&SortBy=DateCreated&SortOrder=Descending&StartIndex=0&Limit=60

SearchOrder accepts SortBy. The allowed sort columns for this extension are SortName, Name, DateCreated, PremiereDate and ProductionYear. Directions are Ascending and Descending. An invalid extension, unsupported column or invalid direction fails the request. The query applies the requested order to the complete authorized match set in SQL before pagination. Item identity breaks ties.

StudioIds, Tags, Genres, Years, IsPlayed, IsFavorite, ParentId and IncludeItemTypes remain ordinary server filters. Every subsequent page and page rehydration must retain the same filters and ordering. Clients reset their scroll/request generation when any query or filter changes.

This extension does not establish support in other Jellyfin-compatible servers. Clients must not describe an ignored extension or a local sort of one fetched page as a complete-library ordering. Public catalogs and provider discovery keep their own ordering and are presented separately.
