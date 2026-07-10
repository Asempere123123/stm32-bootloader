MEMORY
{
  FLASH : ORIGIN = 0x08000000, LENGTH = {{ flash-size }}K
  RAM   : ORIGIN = 0x20000000, LENGTH = 20K
}

{% if external-flash-addr == "NONE" -%}
_app_vector_table = ORIGIN(FLASH) + LENGTH(FLASH);
{% else -%}
_app_vector_table = {{ external-flash-addr }};
{% endif -%}
