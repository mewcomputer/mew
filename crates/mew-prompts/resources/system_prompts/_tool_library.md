{#- Tool library partial. Conditionally included when the caller passes a
    `tool_library` variable (a list of objects with name/description).
    Currently mew does not pass this — the section is a no-op until wired. -#}
{%- if tool_library %}

## Tool Library

The following tools are available in this prompt context. Use the documented field names exactly when calling a tool.
{%- for tool in tool_library %}

### `{{ tool.name }}`

{% if tool.description %}{{ tool.description }}{% else %}No short description available.{% endif %}
{%- endfor %}
{%- endif %}
